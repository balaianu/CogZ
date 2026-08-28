//! Graph traversal — multi-hop edge following.

use std::collections::HashSet;

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
}
