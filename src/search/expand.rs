//! Graph-aware expansion from search results.
//!
//! For each seed entity (a direct search match), follows edges outward
//! via BFS, recording the path from the seed to each discovered entity.
//! This provides provenance: the agent can see *why* an entity was
//! included in the results.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::storage::StorageError;
use crate::storage::graph::get_edges_involving_batch;

/// A graph-expanded entity with its provenance path.
#[derive(Debug, Clone)]
pub struct ExpansionResult {
    pub entity_id: String,
    /// Path from the seed to this entity: [seed_id, hop1, hop2, ..., this_id]
    pub graph_path: Vec<String>,
    /// Which seed entity this was expanded from.
    pub seed_id: String,
}

/// Expand from seed entities via BFS, recording paths.
///
/// Follows both outgoing and incoming edges. Each seed gets its own
/// BFS — entities discovered from different seeds get separate results
/// with their respective paths. Entities already in `exclude_ids` are
/// not returned (prevents duplicating direct search matches).
///
/// Only entities matching `status_filter` are included in results.
pub fn expand_with_paths(
    conn: &Connection,
    seed_ids: &[String],
    max_hops: usize,
    exclude_ids: &HashSet<String>,
    status_filter: Option<&str>,
) -> Result<Vec<ExpansionResult>, StorageError> {
    if max_hops == 0 || seed_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    for seed_id in seed_ids {
        let expansions = bfs_from_seed(conn, seed_id, max_hops, exclude_ids, status_filter)?;
        results.extend(expansions);
    }

    Ok(results)
}

/// BFS from a single seed, recording the path to each discovered entity.
///
/// Uses `get_edges_involving_batch` to fetch all edges touching the
/// frontier in a single query, then determines parent-child
/// relationships in Rust — no per-neighbor SQL queries.
fn bfs_from_seed(
    conn: &Connection,
    seed_id: &str,
    max_hops: usize,
    exclude_ids: &HashSet<String>,
    status_filter: Option<&str>,
) -> Result<Vec<ExpansionResult>, StorageError> {
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(seed_id.to_string());

    // Map entity_id → path from seed
    let mut paths: HashMap<String, Vec<String>> = HashMap::new();
    paths.insert(seed_id.to_string(), vec![seed_id.to_string()]);

    let mut frontier = vec![seed_id.to_string()];
    let mut discovered = Vec::new();

    for _ in 0..max_hops {
        // Single query: all edges where either endpoint is in the frontier
        let edges = get_edges_involving_batch(conn, &frontier)?;
        if edges.is_empty() {
            break;
        }

        // Build neighbor → parent_id map from the edge list.
        // For each edge (source, target, _type), if one endpoint is in
        // the frontier and the other isn't visited yet, the frontier
        // endpoint is the parent.
        let frontier_set: HashSet<&String> = frontier.iter().collect();
        let mut next_neighbors: Vec<String> = Vec::new();

        for (source, target, _) in &edges {
            let (parent, neighbor) = if frontier_set.contains(source) {
                (source, target)
            } else if frontier_set.contains(target) {
                (target, source)
            } else {
                continue;
            };

            if visited.insert(neighbor.clone()) {
                let parent_path = paths
                    .get(parent)
                    .cloned()
                    .ok_or(StorageError::EntityNotFound(parent.clone()))?;
                let mut path = parent_path;
                path.push(neighbor.clone());
                paths.insert(neighbor.clone(), path.clone());

                if !exclude_ids.contains(neighbor) {
                    let include = match status_filter {
                        None | Some("all") => true,
                        Some(status) => entity_has_status(conn, neighbor, status)?,
                    };
                    if include {
                        discovered.push(ExpansionResult {
                            entity_id: neighbor.clone(),
                            graph_path: path,
                            seed_id: seed_id.to_string(),
                        });
                    }
                }

                next_neighbors.push(neighbor.clone());
            }
        }

        if next_neighbors.is_empty() {
            break;
        }
        frontier = next_neighbors;
    }

    Ok(discovered)
}

/// Check if an entity has the given status.
fn entity_has_status(
    conn: &Connection,
    entity_id: &str,
    status: &str,
) -> Result<bool, StorageError> {
    let actual: Option<String> = conn
        .query_row(
            "SELECT status FROM entities WHERE id = ?1",
            rusqlite::params![entity_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(StorageError::from(other)),
        })?;
    Ok(actual.as_deref() == Some(status))
}

#[cfg(test)]
mod tests {
    use super::super::super::storage::crud::Entity;
    use super::super::super::storage::crud::insert_entity;
    use super::super::super::storage::edges::Edge;
    use super::super::super::storage::edges::insert_edge;
    use super::super::super::storage::ensure_vec_extension;
    use super::super::super::storage::schema::run_migrations;
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
    fn expand_1_hop() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("obs1", "observation", "Bug Report", "c"),
        )
        .unwrap();
        insert_entity(&conn, &Entity::new("func1", "function", "build_sql", "c")).unwrap();

        insert_edge(&conn, &edge("obs1", "func1", "references")).unwrap();

        let exclude = HashSet::new();
        let results = expand_with_paths(&conn, &["obs1".to_string()], 1, &exclude, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, "func1");
        assert_eq!(results[0].graph_path, vec!["obs1", "func1"]);
        assert_eq!(results[0].seed_id, "obs1");
    }

    #[test]
    fn expand_2_hops() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("obs1", "observation", "Bug", "c")).unwrap();
        insert_entity(&conn, &Entity::new("func1", "function", "build_sql", "c")).unwrap();
        insert_entity(&conn, &Entity::new("func2", "function", "search_all", "c")).unwrap();

        insert_edge(&conn, &edge("obs1", "func1", "references")).unwrap();
        insert_edge(&conn, &edge("func1", "func2", "calls")).unwrap();

        let exclude = HashSet::new();
        let results = expand_with_paths(&conn, &["obs1".to_string()], 2, &exclude, None).unwrap();

        assert_eq!(results.len(), 2);
        let func2 = results.iter().find(|r| r.entity_id == "func2").unwrap();
        assert_eq!(func2.graph_path, vec!["obs1", "func1", "func2"]);
    }

    #[test]
    fn expand_excludes_specified_ids() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("obs1", "observation", "Bug", "c")).unwrap();
        insert_entity(&conn, &Entity::new("func1", "function", "build_sql", "c")).unwrap();

        insert_edge(&conn, &edge("obs1", "func1", "references")).unwrap();

        let mut exclude = HashSet::new();
        exclude.insert("func1".to_string());

        let results = expand_with_paths(&conn, &["obs1".to_string()], 1, &exclude, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn expand_no_edges() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("obs1", "observation", "Bug", "c")).unwrap();

        let exclude = HashSet::new();
        let results = expand_with_paths(&conn, &["obs1".to_string()], 2, &exclude, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn expand_zero_hops() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("obs1", "observation", "Bug", "c")).unwrap();

        let exclude = HashSet::new();
        let results = expand_with_paths(&conn, &["obs1".to_string()], 0, &exclude, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn expand_filters_by_status() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("obs1", "observation", "Bug", "c")).unwrap();
        let mut stale = Entity::new("func1", "function", "build_sql", "c");
        stale.status = "stale".to_string();
        insert_entity(&conn, &stale).unwrap();

        insert_edge(&conn, &edge("obs1", "func1", "references")).unwrap();

        let exclude = HashSet::new();
        let results =
            expand_with_paths(&conn, &["obs1".to_string()], 1, &exclude, Some("active")).unwrap();
        assert!(results.is_empty());

        let results =
            expand_with_paths(&conn, &["obs1".to_string()], 1, &exclude, Some("stale")).unwrap();
        assert_eq!(results.len(), 1);
    }
}
