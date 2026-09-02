//! Entity access tracking — derived state for composite scoring.
//!
//! Tracks how often entities appear in search results and context
//! packs. Not in canonical files; resets to 0 on DB rebuild.

use rusqlite::Connection;

use super::StorageError;

/// Increment access counts for a batch of entity IDs. Uses INSERT OR
/// IGNORE + UPDATE to handle both first-access and subsequent accesses
/// in a single SQL statement per entity.
pub fn increment_access_batch(
    conn: &Connection,
    entity_ids: &[String],
) -> Result<(), StorageError> {
    if entity_ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "INSERT INTO entity_access (entity_id, access_count, last_accessed)
         VALUES (?1, 1, ?2)
         ON CONFLICT(entity_id) DO UPDATE SET
             access_count = access_count + 1,
             last_accessed = ?2",
    )?;
    for id in entity_ids {
        stmt.execute(rusqlite::params![id, now])?;
    }
    Ok(())
}

/// Get access counts for a batch of entity IDs. Returns a map of
/// entity_id → access_count. Entities with no access record default to 0.
pub fn get_access_counts_batch(
    conn: &Connection,
    entity_ids: &[String],
) -> Result<std::collections::HashMap<String, i64>, StorageError> {
    if entity_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let placeholders = (0..entity_ids.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT entity_id, access_count FROM entity_access WHERE entity_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = entity_ids
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    let rows = stmt.query_map(params.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut counts = std::collections::HashMap::new();
    for row in rows {
        let (id, count) = row?;
        counts.insert(id, count);
    }
    Ok(counts)
}

/// Get top entities by access count. Returns (entity_id, access_count)
/// pairs sorted by descending access_count. Only includes entities with
/// access_count > 0.
pub fn top_by_access_count(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<(String, i64)>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT entity_id, access_count FROM entity_access
         WHERE access_count > 0
         ORDER BY access_count DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![limit as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::crud::{Entity, insert_entity};
    use crate::storage::schema::run_migrations;

    fn setup() -> Connection {
        crate::storage::ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();
        conn
    }

    #[test]
    fn increment_and_retrieve() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("e1", "observation", "T", "c")).unwrap();
        insert_entity(&conn, &Entity::new("e2", "observation", "T2", "c")).unwrap();

        increment_access_batch(&conn, &["e1".to_string()]).unwrap();
        increment_access_batch(&conn, &["e1".to_string(), "e2".to_string()]).unwrap();

        let counts = get_access_counts_batch(&conn, &["e1".to_string(), "e2".to_string()]).unwrap();
        assert_eq!(counts.get("e1"), Some(&2));
        assert_eq!(counts.get("e2"), Some(&1));
    }

    #[test]
    fn missing_entity_returns_zero() {
        let conn = setup();
        let counts = get_access_counts_batch(&conn, &["nonexistent".to_string()]).unwrap();
        assert!(counts.is_empty());
    }

    #[test]
    fn top_by_access_orders_descending() {
        let conn = setup();
        for i in 0..5 {
            let id = format!("e{i}");
            insert_entity(&conn, &Entity::new(&id, "observation", "T", "c")).unwrap();
            // Access e0 once, e1 twice, etc.
            for _ in 0..=i {
                increment_access_batch(&conn, std::slice::from_ref(&id)).unwrap();
            }
        }

        let top = top_by_access_count(&conn, 3).unwrap();
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].0, "e4");
        assert_eq!(top[0].1, 5);
        assert_eq!(top[1].0, "e3");
        assert_eq!(top[2].0, "e2");
    }
}
