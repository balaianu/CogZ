//! Embedding operations — vec0 vector storage for semantic search.
//!
//! Code and knowledge entities use different embedding models with
//! incompatible vector spaces. Each space has its own vec0 table so
//! KNN only compares vectors within the same model space.

use rusqlite::{Connection, params};
use zerocopy::IntoBytes;

use super::StorageError;
use super::crud::EntityType;

/// Which embedding table to operate on — determined by entity type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingSpace {
    Code,
    Knowledge,
}

impl EmbeddingSpace {
    pub fn table(&self) -> &'static str {
        match self {
            Self::Code => "code_embeddings",
            Self::Knowledge => "knowledge_embeddings",
        }
    }

    /// Determine the embedding space from an entity type string.
    /// Returns `None` for invalid types (caller should skip).
    pub fn from_entity_type(entity_type: &str) -> Option<Self> {
        EntityType::parse(entity_type).ok().map(|t| {
            if t.is_code() {
                Self::Code
            } else {
                Self::Knowledge
            }
        })
    }
}

/// Insert an embedding for an entity into the appropriate table.
/// The embedding dimension must match the vec0 table definition.
/// The table is selected based on the entity's type.
pub fn insert_embedding(
    conn: &Connection,
    entity_id: &str,
    entity_type: &str,
    embedding: &[f32],
) -> Result<(), StorageError> {
    let space = EmbeddingSpace::from_entity_type(entity_type)
        .ok_or_else(|| StorageError::InvalidEntityType(entity_type.to_string()))?;
    let sql = format!(
        "INSERT INTO {}(entity_id, embedding) VALUES (?1, ?2)",
        space.table()
    );
    conn.execute(&sql, params![entity_id, embedding.as_bytes()])?;
    Ok(())
}

/// Delete an embedding from both tables (the entity's type may have
/// changed, so we check both). This is safe — only one table will
/// have the row.
pub fn delete_embedding(conn: &Connection, entity_id: &str) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM code_embeddings WHERE entity_id = ?1",
        params![entity_id],
    )?;
    conn.execute(
        "DELETE FROM knowledge_embeddings WHERE entity_id = ?1",
        params![entity_id],
    )?;
    Ok(())
}

/// Get the embedding for a single entity. Checks both tables.
/// Returns `None` if no embedding is stored for the entity.
/// Returns an error if the stored blob is corrupted (not a multiple of 4 bytes).
pub fn get_embedding(conn: &Connection, entity_id: &str) -> Result<Option<Vec<f32>>, StorageError> {
    // Try knowledge first (more common in query paths), then code.
    for table in ["knowledge_embeddings", "code_embeddings"] {
        let sql = format!("SELECT embedding FROM {} WHERE entity_id = ?1", table);
        let mut stmt = conn.prepare(&sql)?;
        let result = stmt.query_row(params![entity_id], |r| {
            let blob: Vec<u8> = r.get(0)?;
            if !blob.len().is_multiple_of(4) {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "embedding blob is {} bytes, not a multiple of 4",
                            blob.len()
                        ),
                    )),
                ));
            }
            let floats: Vec<f32> = blob
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect();
            Ok(floats)
        });
        match result {
            Ok(embedding) => return Ok(Some(embedding)),
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(None)
}

/// Batch-fetch embeddings for multiple entity IDs from the knowledge
/// embeddings table. Returns a map of entity_id → embedding. Entities
/// without embeddings are simply absent from the map.
///
/// This is more efficient than calling `get_embedding` per entity when
/// you need embeddings for a large set of entities (e.g. all active
/// observations during merge candidate detection).
pub fn get_knowledge_embeddings_batch(
    conn: &Connection,
    entity_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<f32>>, StorageError> {
    if entity_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    const CHUNK_SIZE: usize = 998;
    let mut result = std::collections::HashMap::new();

    for chunk in entity_ids.chunks(CHUNK_SIZE) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let sql = format!(
            "SELECT entity_id, embedding FROM knowledge_embeddings WHERE entity_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |r| {
            let id: String = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            if !blob.len().is_multiple_of(4) {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "embedding blob is {} bytes, not a multiple of 4",
                            blob.len()
                        ),
                    )),
                ));
            }
            let floats: Vec<f32> = blob
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect();
            Ok((id, floats))
        })?;
        for row in rows {
            let (id, embedding) = row?;
            result.insert(id, embedding);
        }
    }

    Ok(result)
}

/// Count total embeddings across both tables.
pub fn count_embeddings(conn: &Connection) -> Result<i64, StorageError> {
    let code: i64 = conn.query_row("SELECT COUNT(*) FROM code_embeddings", [], |r| r.get(0))?;
    let knowledge: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge_embeddings", [], |r| {
        r.get(0)
    })?;
    Ok(code + knowledge)
}

/// K-nearest-neighbor search within a specific embedding space.
/// Returns (entity_id, distance) pairs sorted by ascending distance.
pub fn knn_search(
    conn: &Connection,
    space: EmbeddingSpace,
    query: &[f32],
    k: i64,
) -> Result<Vec<(String, f32)>, StorageError> {
    let sql = format!(
        "SELECT entity_id, distance
         FROM {}
         WHERE embedding MATCH ?1 AND k = ?2
         ORDER BY distance",
        space.table()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![query.as_bytes(), k], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?))
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// KNN search filtered by entity type. Uses a subquery to restrict
/// results to entities of the specified type, preventing unrelated
/// entity types from consuming KNN slots. For example, when searching
/// for merge candidates among observations, this prevents knowledge
/// entries from crowding out observation neighbors.
pub fn knn_search_with_type_filter(
    conn: &Connection,
    space: EmbeddingSpace,
    query: &[f32],
    k: i64,
    entity_type: &str,
) -> Result<Vec<(String, f32)>, StorageError> {
    let sql = format!(
        "SELECT entity_id, distance
         FROM {}
         WHERE embedding MATCH ?1 AND k = ?2
           AND entity_id IN (SELECT id FROM entities WHERE type = ?3)
         ORDER BY distance",
        space.table()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![query.as_bytes(), k, entity_type], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?))
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::super::crud::Entity;
    use super::super::crud::insert_entity;
    use super::super::ensure_vec_extension;
    use super::super::schema::run_migrations;
    use super::*;

    fn setup() -> Connection {
        ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();
        conn
    }

    #[test]
    fn insert_and_knn_search_knowledge() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "Test", "content")).unwrap();

        let embedding = vec![0.1_f32; 768];
        insert_embedding(&conn, "u1", "observation", &embedding).unwrap();

        let results = knn_search(&conn, EmbeddingSpace::Knowledge, &embedding, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "u1");
        assert!(results[0].1 < 0.001);
    }

    #[test]
    fn insert_and_knn_search_code() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("f1", "function", "my_func", "fn my_func() {}"),
        )
        .unwrap();

        let embedding = vec![0.2_f32; 768];
        insert_embedding(&conn, "f1", "function", &embedding).unwrap();

        let results = knn_search(&conn, EmbeddingSpace::Code, &embedding, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "f1");
    }

    #[test]
    fn knn_search_isolated_per_space() {
        let conn = setup();
        // Code entity with embedding close to query
        insert_entity(
            &conn,
            &Entity::new("f1", "function", "func", "fn func() {}"),
        )
        .unwrap();
        insert_embedding(&conn, "f1", "function", &vec![0.1_f32; 768]).unwrap();

        // Knowledge entity with embedding far from query
        insert_entity(&conn, &Entity::new("o1", "observation", "obs", "content")).unwrap();
        insert_embedding(&conn, "o1", "observation", &vec![0.9_f32; 768]).unwrap();

        let query = vec![0.1_f32; 768];

        // Code KNN should only find the function
        let code_results = knn_search(&conn, EmbeddingSpace::Code, &query, 10).unwrap();
        assert_eq!(code_results.len(), 1);
        assert_eq!(code_results[0].0, "f1");

        // Knowledge KNN should only find the observation
        let knowledge_results = knn_search(&conn, EmbeddingSpace::Knowledge, &query, 10).unwrap();
        assert_eq!(knowledge_results.len(), 1);
        assert_eq!(knowledge_results[0].0, "o1");
    }

    #[test]
    fn knn_search_returns_nearest() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

        insert_embedding(&conn, "u1", "observation", &vec![0.1_f32; 768]).unwrap();
        insert_embedding(&conn, "u2", "observation", &vec![0.9_f32; 768]).unwrap();

        let query = vec![0.1_f32; 768];
        let results = knn_search(&conn, EmbeddingSpace::Knowledge, &query, 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "u1");
        assert!(results[0].1 < results[1].1);
    }

    #[test]
    fn delete_entity_embedding() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "T", "c")).unwrap();
        insert_embedding(&conn, "u1", "observation", &vec![0.1_f32; 768]).unwrap();

        delete_embedding(&conn, "u1").unwrap();

        let results = knn_search(&conn, EmbeddingSpace::Knowledge, &vec![0.1_f32; 768], 1).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn knn_search_type_filter_excludes_other_types() {
        let conn = setup();
        // Insert an observation and a knowledge entry with identical
        // embeddings. Without the type filter, both would be returned.
        insert_entity(&conn, &Entity::new("obs1", "observation", "Obs", "c")).unwrap();
        insert_entity(&conn, &Entity::new("k1", "knowledge", "Knowledge", "c")).unwrap();
        insert_embedding(&conn, "obs1", "observation", &vec![0.1_f32; 768]).unwrap();
        insert_embedding(&conn, "k1", "knowledge", &vec![0.1_f32; 768]).unwrap();

        let query = vec![0.1_f32; 768];

        // Unfiltered: both entities are returned.
        let unfiltered = knn_search(&conn, EmbeddingSpace::Knowledge, &query, 10).unwrap();
        assert_eq!(unfiltered.len(), 2);

        // Filtered to observations only: knowledge entry is excluded.
        let filtered = knn_search_with_type_filter(
            &conn,
            EmbeddingSpace::Knowledge,
            &query,
            10,
            "observation",
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "obs1");
    }

    #[test]
    fn get_embedding_checks_both_tables() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("f1", "function", "func", "fn func() {}"),
        )
        .unwrap();
        insert_embedding(&conn, "f1", "function", &vec![0.5_f32; 768]).unwrap();

        let emb = get_embedding(&conn, "f1").unwrap();
        assert!(emb.is_some());
        assert_eq!(emb.unwrap().len(), 768);
    }

    #[test]
    fn count_embeddings_across_both_tables() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("f1", "function", "func", "fn func() {}"),
        )
        .unwrap();
        insert_entity(&conn, &Entity::new("o1", "observation", "obs", "content")).unwrap();

        insert_embedding(&conn, "f1", "function", &vec![0.1_f32; 768]).unwrap();
        insert_embedding(&conn, "o1", "observation", &vec![0.2_f32; 768]).unwrap();

        assert_eq!(count_embeddings(&conn).unwrap(), 2);
    }

    #[test]
    fn get_knowledge_embeddings_batch_returns_map() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("k1", "knowledge", "K1", "c")).unwrap();
        insert_entity(&conn, &Entity::new("k2", "knowledge", "K2", "c")).unwrap();
        insert_entity(&conn, &Entity::new("k3", "knowledge", "K3", "c")).unwrap();

        insert_embedding(&conn, "k1", "knowledge", &vec![0.1_f32; 768]).unwrap();
        insert_embedding(&conn, "k2", "knowledge", &vec![0.2_f32; 768]).unwrap();
        // k3 has no embedding

        let map = get_knowledge_embeddings_batch(
            &conn,
            &["k1".to_string(), "k2".to_string(), "k3".to_string()],
        )
        .unwrap();

        assert_eq!(map.len(), 2);
        assert!(map.contains_key("k1"));
        assert!(map.contains_key("k2"));
        assert!(!map.contains_key("k3"));
    }

    #[test]
    fn get_knowledge_embeddings_batch_empty_input() {
        let conn = setup();
        let map = get_knowledge_embeddings_batch(&conn, &[]).unwrap();
        assert!(map.is_empty());
    }
}
