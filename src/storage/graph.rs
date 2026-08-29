//! Graph traversal — multi-hop edge following and batched edge lookup.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::StorageError;
use super::edges::get_neighbors_batch;

/// Graph traversal — get entities N hops away from a starting entity,
/// following edges of any type (both outgoing and incoming).
///
/// Returns a flat list of reachable entity IDs (excluding the start).
pub fn graph_traverse(
    conn: &Connection,
    start_id: &str,
    max_hops: usize,
) -> Result<Vec<String>, StorageError> {
    let mut visited = HashSet::new();
    visited.insert(start_id.to_string());

    let mut frontier = vec![start_id.to_string()];

    for _ in 0..max_hops {
        let (outgoing, incoming) = get_neighbors_batch(conn, &frontier)?;

        let mut next_frontier = Vec::new();
        for target_id in outgoing {
            if visited.insert(target_id.clone()) {
                next_frontier.push(target_id);
            }
        }
        for source_id in incoming {
            if visited.insert(source_id.clone()) {
                next_frontier.push(source_id);
            }
        }

        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    visited.remove(start_id);
    Ok(visited.into_iter().collect())
}

/// Batched edge lookup — get all (source_id, target_id, edge_type) pairs
/// where either endpoint is in the given node set. Single query.
///
/// Use this instead of `get_neighbors_batch` + `get_edge_type_between`
/// when you need to know which frontier node each neighbor came from
/// and what edge type connects them.
///
/// Chunks the query to respect SQLite's variable number limit. Each
/// node_id is bound twice (source + target), so the chunk size is
/// `MAX_VARS / 2`.
pub fn get_edges_involving_batch(
    conn: &Connection,
    node_ids: &[String],
) -> Result<Vec<(String, String, String)>, StorageError> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    // SQLITE_MAX_VARIABLE_NUMBER is 999 by default, 32766 in modern
    // SQLite. Use 500 as a safe conservative chunk size (each id is
    // bound twice → 1000 vars, under the default 999... use 499).
    const MAX_VARS: usize = 999;
    const CHUNK_SIZE: usize = MAX_VARS / 2; // 499

    let mut edges = Vec::new();

    for chunk in node_ids.chunks(CHUNK_SIZE) {
        let placeholders = (0..chunk.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        // Params are bound twice — once for source_id IN (...), once for target_id IN (...)
        let params: Vec<&dyn rusqlite::ToSql> = chunk
            .iter()
            .chain(chunk.iter())
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();

        let sql = format!(
            "SELECT DISTINCT source_id, target_id, edge_type
             FROM edges
             WHERE source_id IN ({placeholders}) OR target_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            edges.push(row?);
        }
    }

    Ok(edges)
}

/// Batched references lookup — get `references` edges for a set of
/// entities in a single query. Returns a map of entity_id → list of
/// referenced entity IDs. Chunks to respect SQLite variable limits.
pub fn get_references_batch(
    conn: &Connection,
    entity_ids: &[String],
) -> Result<HashMap<String, Vec<String>>, StorageError> {
    if entity_ids.is_empty() {
        return Ok(HashMap::new());
    }

    const MAX_VARS: usize = 999;
    const CHUNK_SIZE: usize = MAX_VARS; // each id bound once

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for id in entity_ids {
        map.insert(id.clone(), Vec::new());
    }

    for chunk in entity_ids.chunks(CHUNK_SIZE) {
        let placeholders = (0..chunk.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let params: Vec<&dyn rusqlite::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();

        let sql = format!(
            "SELECT source_id, target_id FROM edges
             WHERE edge_type = 'references' AND source_id IN ({placeholders})"
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (source, target) = row?;
            if let Some(refs) = map.get_mut(&source) {
                refs.push(target);
            }
        }
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::super::crud::Entity;
    use super::super::crud::insert_entity;
    use super::super::edges::Edge;
    use super::super::edges::insert_edge;
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
    fn traverse_1_hop() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u3", "function", "C", "c")).unwrap();

        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        insert_edge(&conn, &edge("u2", "u3", "calls")).unwrap();

        let reachable = graph_traverse(&conn, "u1", 1).unwrap();
        assert!(reachable.contains(&"u2".to_string()));
        assert!(!reachable.contains(&"u3".to_string()));
    }

    #[test]
    fn traverse_2_hops() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u3", "function", "C", "c")).unwrap();

        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        insert_edge(&conn, &edge("u2", "u3", "calls")).unwrap();

        let reachable = graph_traverse(&conn, "u1", 2).unwrap();
        assert!(reachable.contains(&"u2".to_string()));
        assert!(reachable.contains(&"u3".to_string()));
    }

    #[test]
    fn traverse_bidirectional() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

        // u2 references u1 (incoming edge to u1)
        insert_edge(&conn, &edge("u2", "u1", "references")).unwrap();

        let reachable = graph_traverse(&conn, "u1", 1).unwrap();
        assert!(reachable.contains(&"u2".to_string()));
    }

    #[test]
    fn traverse_no_edges() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();

        let reachable = graph_traverse(&conn, "u1", 3).unwrap();
        assert!(reachable.is_empty());
    }

    #[test]
    fn edges_involving_batch_returns_both_directions() {
        let conn = setup();
        for id in &["u1", "u2", "u3"] {
            insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
        }
        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        insert_edge(&conn, &edge("u3", "u1", "calls")).unwrap();

        let edges = get_edges_involving_batch(&conn, &["u1".to_string()]).unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges.contains(&("u1".to_string(), "u2".to_string(), "references".to_string())));
        assert!(edges.contains(&("u3".to_string(), "u1".to_string(), "calls".to_string())));
    }

    #[test]
    fn edges_involving_batch_empty() {
        let conn = setup();
        let edges = get_edges_involving_batch(&conn, &[]).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn references_batch_returns_references_map() {
        let conn = setup();
        for id in &["u1", "u2", "u3", "u4"] {
            insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
        }
        insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
        insert_edge(&conn, &edge("u1", "u3", "references")).unwrap();
        insert_edge(&conn, &edge("u2", "u4", "references")).unwrap();
        insert_edge(&conn, &edge("u3", "u4", "calls")).unwrap();

        assert!(get_references_batch(&conn, &[]).unwrap().is_empty());

        let map = get_references_batch(
            &conn,
            &["u1".to_string(), "u2".to_string(), "u3".to_string()],
        )
        .unwrap();

        let u1_refs = map.get("u1").unwrap();
        assert_eq!(u1_refs.len(), 2);
        assert!(u1_refs.contains(&"u2".to_string()));
        assert!(u1_refs.contains(&"u3".to_string()));

        assert_eq!(map.get("u2").unwrap().len(), 1);
        assert!(map.get("u2").unwrap().contains(&"u4".to_string()));

        assert_eq!(map.get("u3").unwrap().len(), 0);
    }
}
