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

/// KNN search filtered by entity type, status, and test-path exclusion.
/// Pushes all filters into the KNN subquery so non-matching entities
/// don't consume KNN slots. This prevents stale/rejected/superseded
/// embeddings and test entities from starving out active production
/// results in a bounded KNN window.
pub fn knn_search_with_filters(
    conn: &Connection,
    space: EmbeddingSpace,
    query: &[f32],
    k: i64,
    entity_type: Option<&str>,
    status: Option<&str>,
    exclude_tests: bool,
) -> Result<Vec<(String, f32)>, StorageError> {
    let mut where_clause = String::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query.as_bytes()), Box::new(k)];
    let mut next_idx = 3usize;

    if let Some(et) = entity_type {
        where_clause.push_str(&format!(
            " AND entity_id IN (SELECT id FROM entities WHERE type = ?{next_idx})"
        ));
        params.push(Box::new(et.to_string()));
        next_idx += 1;
    }
    if let Some(st) = status {
        where_clause.push_str(&format!(
            " AND entity_id IN (SELECT id FROM entities WHERE status = ?{next_idx})"
        ));
        params.push(Box::new(st.to_string()));
    }
    if exclude_tests {
        // Exclude entities whose file_path looks like a test file.
        // Must match all patterns recognized by
        // `index::gitignore::is_test_file` so that vector search
        // doesn't return test entities in any supported language.
        where_clause.push_str(
            " AND entity_id NOT IN (SELECT id FROM entities WHERE \
             file_path LIKE 'tests/%' OR file_path LIKE '%/tests/%' \
             OR file_path LIKE '%/tests.rs' OR file_path LIKE '%/_tests.rs' \
             OR file_path LIKE '%_test.go' \
             OR file_path LIKE 'test_%.py' OR file_path LIKE '%/test_%.py' \
             OR file_path LIKE '%_test.py' OR file_path LIKE '%/_test.py' \
             OR file_path LIKE '%/__tests__/%' \
             OR file_path LIKE '%.test.js' OR file_path LIKE '%.test.jsx' \
             OR file_path LIKE '%.test.ts' OR file_path LIKE '%.test.tsx' \
             OR file_path LIKE '%.test.mjs' OR file_path LIKE '%.test.cjs' \
             OR file_path LIKE '%.spec.js' OR file_path LIKE '%.spec.jsx' \
             OR file_path LIKE '%.spec.ts' OR file_path LIKE '%.spec.tsx' \
             OR file_path LIKE '%.spec.mjs' OR file_path LIKE '%.spec.cjs' \
             OR file_path LIKE 'test_%.sh' OR file_path LIKE '%/test_%.sh' \
             OR file_path LIKE '%_test.sh' OR file_path LIKE '%/_test.sh' \
             OR file_path LIKE 'test_%.bash' OR file_path LIKE '%/test_%.bash' \
             OR file_path LIKE '%_test.bash' OR file_path LIKE '%/_test.bash')",
        );
    }

    let sql = format!(
        "SELECT entity_id, distance
         FROM {}
         WHERE embedding MATCH ?1 AND k = ?2{}
         ORDER BY distance",
        space.table(),
        where_clause
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?))
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

#[cfg(test)]
#[path = "embeddings_tests.rs"]
mod tests;
