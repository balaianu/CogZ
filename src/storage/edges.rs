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
}
