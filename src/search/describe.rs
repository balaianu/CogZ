//! Batched path description — human-readable provenance strings for
//! graph-expanded search results.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::storage::StorageError;
use crate::storage::graph::get_edges_involving_batch;

/// Build human-readable descriptions for multiple graph paths in batch.
///
/// Fetches all entity titles and all edge types in two queries total,
/// then assembles descriptions in Rust.
pub fn build_path_descriptions_batch(
    conn: &Connection,
    paths: &[Vec<String>],
) -> Result<Vec<String>, StorageError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    // Collect all unique entity IDs and all unique (a, b) consecutive pairs
    let mut all_ids: HashSet<&String> = HashSet::new();
    let mut all_pairs: HashSet<(&String, &String)> = HashSet::new();
    for path in paths {
        if path.len() <= 1 {
            continue;
        }
        for node in path {
            all_ids.insert(node);
        }
        for i in 0..path.len() - 1 {
            all_pairs.insert((&path[i], &path[i + 1]));
        }
    }

    // Batch 1: fetch all titles
    let titles = batch_get_titles(conn, &all_ids)?;

    // Batch 2: fetch all edge types between consecutive pairs
    let edge_types = batch_get_edge_types(conn, &all_pairs)?;

    // Assemble descriptions
    let mut descriptions = Vec::with_capacity(paths.len());
    for path in paths {
        if path.len() <= 1 {
            descriptions.push(String::new());
            continue;
        }
        let mut parts = Vec::with_capacity(path.len() * 2 - 1);
        for i in 0..path.len() {
            parts.push(
                titles
                    .get(&path[i])
                    .cloned()
                    .unwrap_or_else(|| path[i].clone()),
            );
            if i + 1 < path.len() {
                let etype = edge_types
                    .get(&(path[i].clone(), path[i + 1].clone()))
                    .cloned()
                    .unwrap_or_else(|| "→".to_string())
                    .replace('_', " ");
                parts.push(etype);
            }
        }
        descriptions.push(parts.join(" → "));
    }
    Ok(descriptions)
}

/// Batch-fetch titles for a set of entity IDs. Returns a map of
/// ID → title for entities that exist. Missing entities are absent
/// from the map; the caller falls back to the ID itself.
fn batch_get_titles(
    conn: &Connection,
    ids: &HashSet<&String>,
) -> Result<HashMap<String, String>, StorageError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let id_vec: Vec<&String> = ids.iter().copied().collect();
    let placeholders = (0..id_vec.len()).map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<&dyn rusqlite::ToSql> =
        id_vec.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let sql = format!("SELECT id, title FROM entities WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let mut titles = HashMap::new();
    for row in rows {
        let (id, title): (String, Option<String>) = row?;
        titles.insert(id, title.unwrap_or_default());
    }
    Ok(titles)
}

/// Batch-fetch edge types for a set of (a, b) pairs.
fn batch_get_edge_types(
    conn: &Connection,
    pairs: &HashSet<(&String, &String)>,
) -> Result<HashMap<(String, String), String>, StorageError> {
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut result = HashMap::new();
    let mut all_nodes: HashSet<&String> = HashSet::new();
    for (a, b) in pairs {
        all_nodes.insert(a);
        all_nodes.insert(b);
    }
    let node_vec: Vec<String> = all_nodes.iter().map(|s| s.to_string()).collect();
    let edges = get_edges_involving_batch(conn, &node_vec)?;
    for (source, target, etype) in &edges {
        if pairs.contains(&(source, target)) {
            result.insert((source.clone(), target.clone()), etype.clone());
        }
        if pairs.contains(&(target, source)) {
            result.insert((target.clone(), source.clone()), etype.clone());
        }
    }
    Ok(result)
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
    fn path_descriptions_batch_includes_titles_and_edge_types() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("obs1", "observation", "Bug Report", "c"),
        )
        .unwrap();
        insert_entity(&conn, &Entity::new("func1", "function", "build_sql", "c")).unwrap();

        insert_edge(&conn, &edge("obs1", "func1", "references")).unwrap();

        let paths = vec![vec!["obs1".to_string(), "func1".to_string()]];
        let descs = build_path_descriptions_batch(&conn, &paths).unwrap();
        assert_eq!(descs.len(), 1);
        assert!(descs[0].contains("Bug Report"));
        assert!(descs[0].contains("references"));
        assert!(descs[0].contains("build_sql"));
    }

    #[test]
    fn path_descriptions_batch_empty_for_single_node() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("obs1", "observation", "Bug", "c")).unwrap();

        let paths = vec![vec!["obs1".to_string()]];
        let descs = build_path_descriptions_batch(&conn, &paths).unwrap();
        assert_eq!(descs.len(), 1);
        assert!(descs[0].is_empty());
    }

    #[test]
    fn path_descriptions_batch_multiple_paths() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("a", "observation", "Alpha", "c")).unwrap();
        insert_entity(&conn, &Entity::new("b", "function", "Beta", "c")).unwrap();
        insert_entity(&conn, &Entity::new("c", "function", "Gamma", "c")).unwrap();

        insert_edge(&conn, &edge("a", "b", "references")).unwrap();
        insert_edge(&conn, &edge("a", "c", "calls")).unwrap();

        let paths = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["a".to_string(), "c".to_string()],
        ];
        let descs = build_path_descriptions_batch(&conn, &paths).unwrap();
        assert_eq!(descs.len(), 2);
        assert!(descs[0].contains("Alpha"));
        assert!(descs[0].contains("Beta"));
        assert!(descs[1].contains("Gamma"));
    }
}
