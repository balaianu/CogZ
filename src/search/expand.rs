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

/// Expand from seed entities via fused multi-seed BFS, recording paths.
///
/// All seeds are placed in the initial frontier and expanded together.
/// This means one `get_edges_involving_batch` query per hop instead of
/// one per hop per seed. Each discovered entity records which seed it
/// was reached from and the path from that seed.
///
/// Follows both outgoing and incoming edges. Entities already in
/// `exclude_ids` are not returned (prevents duplicating direct search
/// matches). Only entities matching `status_filter` are included.
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

    // Global visited set across all seeds — an entity discovered from
    // one seed is not rediscovered from another.
    let mut visited: HashSet<String> = HashSet::new();
    for seed_id in seed_ids {
        visited.insert(seed_id.clone());
    }

    // Map entity_id → (path from seed, seed_id)
    let mut paths: HashMap<String, (Vec<String>, String)> = HashMap::new();
    for seed_id in seed_ids {
        paths.insert(seed_id.clone(), (vec![seed_id.clone()], seed_id.clone()));
    }

    // Initial frontier: all seeds
    let mut frontier: Vec<String> = seed_ids.to_vec();
    let mut discovered = Vec::new();

    for _ in 0..max_hops {
        if frontier.is_empty() {
            break;
        }

        // Single query: all edges where either endpoint is in the frontier
        let edges = get_edges_involving_batch(conn, &frontier)?;
        if edges.is_empty() {
            break;
        }

        let frontier_set: HashSet<&String> = frontier.iter().collect();
        let mut next_neighbors: Vec<String> = Vec::new();
        let mut candidates: Vec<(String, Vec<String>, String)> = Vec::new();

        for (source, target, _) in &edges {
            let (parent, neighbor) = if frontier_set.contains(source) {
                (source, target)
            } else if frontier_set.contains(target) {
                (target, source)
            } else {
                continue;
            };

            if visited.insert(neighbor.clone()) {
                let (parent_path, seed_id) = paths
                    .get(parent)
                    .cloned()
                    .ok_or(StorageError::EntityNotFound(parent.clone()))?;
                let mut path = parent_path;
                path.push(neighbor.clone());
                paths.insert(neighbor.clone(), (path.clone(), seed_id.clone()));

                if !exclude_ids.contains(neighbor) {
                    candidates.push((neighbor.clone(), path, seed_id));
                }

                next_neighbors.push(neighbor.clone());
            }
        }

        // Batch status check: one query for all candidates in this hop
        if !candidates.is_empty() {
            match status_filter {
                None | Some("all") => {
                    for (id, path, seed_id) in candidates {
                        discovered.push(ExpansionResult {
                            entity_id: id,
                            graph_path: path,
                            seed_id,
                        });
                    }
                }
                Some(status) => {
                    let ids: Vec<String> = candidates.iter().map(|(id, _, _)| id.clone()).collect();
                    let matching = batch_check_status(conn, &ids, status)?;
                    let include_set: HashSet<&String> = matching.iter().collect();
                    for (id, path, seed_id) in candidates {
                        if include_set.contains(&id) {
                            discovered.push(ExpansionResult {
                                entity_id: id,
                                graph_path: path,
                                seed_id,
                            });
                        }
                    }
                }
            }
        }

        frontier = next_neighbors;
    }

    Ok(discovered)
}

/// Batch-check which entity IDs have the given status. Returns the
/// subset of `ids` whose status matches. Chunks the query to respect
/// SQLite's variable number limit (one extra var for status).
fn batch_check_status(
    conn: &Connection,
    ids: &[String],
    status: &str,
) -> Result<Vec<String>, StorageError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // SQLITE_MAX_VARIABLE_NUMBER is 999 by default. Each id is one var,
    // plus one for the status parameter → chunk at 998.
    const MAX_VARS: usize = 999;
    const CHUNK_SIZE: usize = MAX_VARS - 1; // 998

    let mut matching = Vec::new();

    for chunk in ids.chunks(CHUNK_SIZE) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let mut params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        params.push(&status);
        let sql = format!("SELECT id FROM entities WHERE id IN ({placeholders}) AND status = ?");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))?;
        for row in rows {
            matching.push(row?);
        }
    }

    Ok(matching)
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
        run_migrations(&conn, 768).unwrap();
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
