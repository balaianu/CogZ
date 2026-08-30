//! Embedding operations — vec0 vector storage for semantic search.

use rusqlite::{Connection, params};
use zerocopy::IntoBytes;

use super::StorageError;

/// Insert an embedding for an entity. The embedding dimension must
/// match the vec0 table definition (768).
pub fn insert_embedding(
    conn: &Connection,
    entity_id: &str,
    embedding: &[f32],
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO entity_embeddings(entity_id, embedding) VALUES (?1, ?2)",
        params![entity_id, embedding.as_bytes()],
    )?;
    Ok(())
}

/// Delete all embeddings for an entity.
pub fn delete_embedding(conn: &Connection, entity_id: &str) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM entity_embeddings WHERE entity_id = ?1",
        params![entity_id],
    )?;
    Ok(())
}

/// Get the embedding for a single entity. Returns `None` if no
/// embedding is stored for the entity.
pub fn get_embedding(conn: &Connection, entity_id: &str) -> Result<Option<Vec<f32>>, StorageError> {
    let mut stmt = conn.prepare("SELECT embedding FROM entity_embeddings WHERE entity_id = ?1")?;
    let result = stmt.query_row(params![entity_id], |r| {
        let blob: Vec<u8> = r.get(0)?;
        let floats: Vec<f32> = blob
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect();
        Ok(floats)
    });
    match result {
        Ok(embedding) => Ok(Some(embedding)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Count total embeddings stored.
pub fn count_embeddings(conn: &Connection) -> Result<i64, StorageError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM entity_embeddings", [], |r| r.get(0))?)
}

/// K-nearest-neighbor search. Returns (entity_id, distance) pairs
/// sorted by ascending distance.
pub fn knn_search(
    conn: &Connection,
    query: &[f32],
    k: i64,
) -> Result<Vec<(String, f32)>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT entity_id, distance
         FROM entity_embeddings
         WHERE embedding MATCH ?1 AND k = ?2
         ORDER BY distance",
    )?;
    let rows = stmt.query_map(params![query.as_bytes(), k], |r| {
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
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_knn_search() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "Test", "content")).unwrap();

        let embedding = vec![0.1_f32; 768];
        insert_embedding(&conn, "u1", &embedding).unwrap();

        let results = knn_search(&conn, &embedding, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "u1");
        assert!(results[0].1 < 0.001); // distance to self ~0
    }

    #[test]
    fn knn_search_returns_nearest() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

        // u1 is close to [0.1, ...], u2 is far
        insert_embedding(&conn, "u1", &vec![0.1_f32; 768]).unwrap();
        insert_embedding(&conn, "u2", &vec![0.9_f32; 768]).unwrap();

        let query = vec![0.1_f32; 768];
        let results = knn_search(&conn, &query, 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "u1"); // nearest first
        assert!(results[0].1 < results[1].1);
    }

    #[test]
    fn delete_entity_embedding() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "T", "c")).unwrap();
        insert_embedding(&conn, "u1", &vec![0.1_f32; 768]).unwrap();

        delete_embedding(&conn, "u1").unwrap();

        let results = knn_search(&conn, &vec![0.1_f32; 768], 1).unwrap();
        assert!(results.is_empty());
    }
}
