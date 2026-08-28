//! Edge CRUD operations — graph relationships between entities.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::StorageError;

/// A row in the `edges` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
    pub weight: f64,
    pub created_at: String,
}

/// Insert an edge. If the edge already exists (same source, target,
/// type), it is replaced (upsert).
pub fn insert_edge(conn: &Connection, edge: &Edge) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO edges (source_id, target_id, edge_type, weight, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            edge.source_id,
            edge.target_id,
            edge.edge_type,
            edge.weight,
            edge.created_at,
        ],
    )?;
    Ok(())
}

/// Delete an edge.
pub fn delete_edge(
    conn: &Connection,
    source_id: &str,
    target_id: &str,
    edge_type: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM edges WHERE source_id = ?1 AND target_id = ?2 AND edge_type = ?3",
        params![source_id, target_id, edge_type],
    )?;
    Ok(())
}

/// Delete all edges of a given type from a source entity.
pub fn delete_edges_by_source_and_type(
    conn: &Connection,
    source_id: &str,
    edge_type: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM edges WHERE source_id = ?1 AND edge_type = ?2",
        params![source_id, edge_type],
    )?;
    Ok(())
}

/// Insert an edge, silently skipping FK constraint violations.
///
/// Used during file sync when a reference points to an entity that
/// hasn't been synced yet. The edge will be created when the target
/// entity is synced and its references are processed.
pub fn insert_edge_skip_fk_violation(conn: &Connection, edge: &Edge) -> Result<(), StorageError> {
    let result = conn.execute(
        "INSERT OR IGNORE INTO edges (source_id, target_id, edge_type, weight, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            edge.source_id,
            edge.target_id,
            edge.edge_type,
            edge.weight,
            edge.created_at,
        ],
    );
    if let Err(e) = result
        && !matches!(
            &e,
            rusqlite::Error::SqliteFailure(ffi, _)
                if ffi.code == rusqlite::ffi::ErrorCode::ConstraintViolation
        )
    {
        return Err(e.into());
    }
    Ok(())
}

/// Get the edge type between two entities, checking both directions.
/// Returns `Some(edge_type)` if an edge exists (either a→b or b→a),
/// `None` if no edge connects them.
pub fn get_edge_type_between(
    conn: &Connection,
    a: &str,
    b: &str,
) -> Result<Option<String>, StorageError> {
    conn.query_row(
        "SELECT edge_type FROM edges
         WHERE (source_id = ?1 AND target_id = ?2)
            OR (source_id = ?2 AND target_id = ?1)
         LIMIT 1",
        params![a, b],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.into()),
    })
}

/// Count all edges.
pub fn count_edges(conn: &Connection) -> Result<i64, StorageError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    Ok(count)
}

/// Get all edges from a source entity.
pub fn get_edges_from(conn: &Connection, source_id: &str) -> Result<Vec<Edge>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT source_id, target_id, edge_type, weight, created_at
         FROM edges WHERE source_id = ?1 ORDER BY edge_type",
    )?;
    let rows = stmt.query_map(params![source_id], |row| {
        Ok(Edge {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            edge_type: row.get(2)?,
            weight: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    let mut edges = Vec::new();
    for row in rows {
        edges.push(row?);
    }
    Ok(edges)
}

/// Get all edges to a target entity.
pub fn get_edges_to(conn: &Connection, target_id: &str) -> Result<Vec<Edge>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT source_id, target_id, edge_type, weight, created_at
         FROM edges WHERE target_id = ?1 ORDER BY edge_type",
    )?;
    let rows = stmt.query_map(params![target_id], |row| {
        Ok(Edge {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            edge_type: row.get(2)?,
            weight: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    let mut edges = Vec::new();
    for row in rows {
        edges.push(row?);
    }
    Ok(edges)
}

/// Batched neighbor lookup — get all node IDs directly connected to
/// any of the given node IDs, in a single query per direction.
///
/// Returns `(outgoing_targets, incoming_sources)` where
/// `outgoing_targets` are nodes reachable via outgoing edges from
/// the frontier, and `incoming_sources` are nodes with edges pointing
/// into the frontier. Use this instead of calling `get_edges_from` /
/// `get_edges_to` per node when traversing a frontier.
pub fn get_neighbors_batch(
    conn: &Connection,
    node_ids: &[String],
) -> Result<(Vec<String>, Vec<String>), StorageError> {
    if node_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let placeholders = (0..node_ids.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let params: Vec<&dyn rusqlite::ToSql> =
        node_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

    let sql_out =
        format!("SELECT DISTINCT target_id FROM edges WHERE source_id IN ({placeholders})");
    let outgoing: Vec<String> = {
        let mut stmt = conn.prepare(&sql_out)?;
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row?);
        }
        v
    };

    let sql_in =
        format!("SELECT DISTINCT source_id FROM edges WHERE target_id IN ({placeholders})");
    let incoming: Vec<String> = {
        let mut stmt = conn.prepare(&sql_in)?;
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row?);
        }
        v
    };

    Ok((outgoing, incoming))
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

    fn edge(source: &str, target: &str, edge_type: &str) -> Edge {
        Edge {
            source_id: source.to_string(),
            target_id: target.to_string(),
            edge_type: edge_type.to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn insert_and_count_edges() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        assert_eq!(count_edges(&conn).unwrap(), 1);
    }

    #[test]
    fn get_edges_from_and_to() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();

        let from = get_edges_from(&conn, "u1").unwrap();
        assert_eq!(from.len(), 1);
        assert_eq!(from[0].target_id, "u2");

        let to = get_edges_to(&conn, "u2").unwrap();
        assert_eq!(to.len(), 1);
        assert_eq!(to[0].source_id, "u1");
    }

    #[test]
    fn remove_edge() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        assert_eq!(count_edges(&conn).unwrap(), 1);

        delete_edge(&conn, "u1", "u2", "references").unwrap();
        assert_eq!(count_edges(&conn).unwrap(), 0);
    }

    #[test]
    fn neighbors_batch_returns_both_directions() {
        let conn = setup();
        for id in &["u1", "u2", "u3", "u4"] {
            insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
        }
        // u1 → u2 (outgoing from u1)
        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        // u3 → u1 (incoming to u1)
        insert_edge(&conn, &edge("u3", "u1", "references")).unwrap();
        // u2 → u3 (outgoing from u2, incoming to u3)

        let (outgoing, incoming) = get_neighbors_batch(&conn, &["u1".to_string()]).unwrap();
        assert!(outgoing.contains(&"u2".to_string()));
        assert!(incoming.contains(&"u3".to_string()));
    }

    #[test]
    fn neighbors_batch_multi_node() {
        let conn = setup();
        for id in &["u1", "u2", "u3", "u4", "u5"] {
            insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
        }
        insert_edge(&conn, &edge("u1", "u4", "references")).unwrap();
        insert_edge(&conn, &edge("u2", "u5", "references")).unwrap();
        insert_edge(&conn, &edge("u3", "u1", "references")).unwrap();

        let frontier = vec!["u1".to_string(), "u2".to_string()];
        let (outgoing, incoming) = get_neighbors_batch(&conn, &frontier).unwrap();
        assert!(outgoing.contains(&"u4".to_string()));
        assert!(outgoing.contains(&"u5".to_string()));
        assert!(incoming.contains(&"u3".to_string()));
    }

    #[test]
    fn neighbors_batch_empty_frontier() {
        let conn = setup();
        let (outgoing, incoming) = get_neighbors_batch(&conn, &[]).unwrap();
        assert!(outgoing.is_empty());
        assert!(incoming.is_empty());
    }

    #[test]
    fn edge_type_between_outgoing() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();
        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();

        assert_eq!(
            get_edge_type_between(&conn, "u1", "u2").unwrap(),
            Some("references".to_string())
        );
    }

    #[test]
    fn edge_type_between_incoming() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();
        insert_edge(&conn, &edge("u2", "u1", "references")).unwrap();

        assert_eq!(
            get_edge_type_between(&conn, "u1", "u2").unwrap(),
            Some("references".to_string())
        );
    }

    #[test]
    fn edge_type_between_none() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

        assert_eq!(get_edge_type_between(&conn, "u1", "u2").unwrap(), None);
    }
}
