//! Structural edge resolution and sync — calls, imports, extends, contains.
//!
//! Resolves raw edges (name-based references from the single-pass AST
//! walk) to UUID-based `CodeEdge` values using the global name→UUID
//! map built from all parsed entities, then syncs them to the DB.

mod incremental;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::index::sync::code_entity_uuid;
use crate::index::tree_sitter::{CodeEntity, RawEdge};
use crate::storage;
use crate::storage::edges::Edge;

/// A structural edge between two code entities.
#[derive(Debug, Clone)]
pub struct CodeEdge {
    pub source_id: String,
    pub target_id: String,
    pub edge_type: &'static str,
}

pub use incremental::sync_code_edges_incremental;

/// Build the global name → UUID map from parsed entities.
///
/// Maps both qualified names and simple names (last segment after
/// `::` or `.`) so that cross-file calls can be resolved by either
/// the full path or the short name.
pub fn build_name_map(
    entities_by_file: &[(String, Vec<CodeEntity>)],
) -> (HashMap<String, String>, HashMap<String, Vec<String>>) {
    let mut name_to_uuid: HashMap<String, String> = HashMap::new();
    let mut file_to_children: HashMap<String, Vec<String>> = HashMap::new();

    for (file_path_str, entities) in entities_by_file {
        for ce in entities {
            let qualified_name = if ce.entity_type == "file" {
                file_path_str.clone()
            } else {
                ce.properties
                    .get("qualified_name")
                    .and_then(|v| v.as_str())
                    .or_else(|| ce.properties.get("module_path").and_then(|v| v.as_str()))
                    .unwrap_or(&ce.title)
                    .to_string()
            };

            let id = code_entity_uuid(file_path_str, ce.entity_type, &qualified_name);

            name_to_uuid.insert(qualified_name.clone(), id.clone());
            if let Some(simple) = qualified_name.rsplit("::").next() {
                name_to_uuid
                    .entry(simple.to_string())
                    .or_insert_with(|| id.clone());
            }
            // Python uses dot separators in qualified names.
            if let Some(simple) = qualified_name.rsplit('.').next() {
                name_to_uuid
                    .entry(simple.to_string())
                    .or_insert_with(|| id.clone());
            }

            if ce.entity_type == "function" || ce.entity_type == "class" {
                file_to_children
                    .entry(file_path_str.clone())
                    .or_default()
                    .push(id);
            }
        }
    }

    (name_to_uuid, file_to_children)
}

/// Resolve raw edges to UUID-based CodeEdges using the name map.
///
/// Tries the full target name first, then the last segment (after
/// `::` or `.`) as a fallback. Unresolved edges are silently dropped.
pub fn resolve_edges(
    raw_edges: &[RawEdge],
    name_to_uuid: &HashMap<String, String>,
) -> Vec<CodeEdge> {
    let mut edges = Vec::new();
    for raw in raw_edges {
        let source_id = name_to_uuid.get(&raw.source_name);
        let target_id = name_to_uuid
            .get(&raw.target_name)
            .or_else(|| {
                raw.target_name
                    .rsplit("::")
                    .next()
                    .and_then(|last| name_to_uuid.get(last))
            })
            .or_else(|| {
                raw.target_name
                    .rsplit('.')
                    .next()
                    .and_then(|last| name_to_uuid.get(last))
            });

        if let (Some(src), Some(tgt)) = (source_id, target_id) {
            edges.push(CodeEdge {
                source_id: src.clone(),
                target_id: tgt.clone(),
                edge_type: raw.edge_type,
            });
        }
    }
    edges
}

/// Build `contains` edges from file entities to their functions/classes.
pub fn build_contains_edges(file_to_children: &HashMap<String, Vec<String>>) -> Vec<CodeEdge> {
    let mut edges = Vec::new();
    for (file_path, children) in file_to_children {
        let file_uuid = code_entity_uuid(file_path, "file", file_path);
        for child_id in children {
            edges.push(CodeEdge {
                source_id: file_uuid.clone(),
                target_id: child_id.clone(),
                edge_type: "contains",
            });
        }
    }
    edges
}

/// Sync structural edges from pre-parsed entities and raw edges.
///
/// Must be called after `sync_code_entities` so that all entities
/// exist in the DB for FK constraints.
///
/// **Full scan only.** This function deletes ALL structural edges
/// and rebuilds from the given data. For incremental reindex,
/// use `sync_code_edges_incremental` instead.
pub fn sync_code_edges(
    storage: &storage::Storage,
    entities_by_file: &[(String, Vec<CodeEntity>)],
    raw_edges_by_file: &[(String, Vec<RawEdge>)],
) {
    // Phase 1: build name → UUID map and file → children map.
    let (name_to_uuid, file_to_children) = build_name_map(entities_by_file);

    // Phase 2: resolve raw edges to UUID-based edges.
    let mut edges: Vec<CodeEdge> = Vec::new();
    for (_, raw_edges) in raw_edges_by_file {
        edges.extend(resolve_edges(raw_edges, &name_to_uuid));
    }

    // Phase 2b: build `contains` edges.
    edges.extend(build_contains_edges(&file_to_children));

    // Phase 3: sync edges to DB. The DELETE and INSERTs are in a
    // single transaction so a COMMIT failure rolls back both —
    // otherwise the DELETE would persist (autocommit) while the
    // INSERTs roll back, silently destroying the graph.
    let conn = storage.conn();

    let in_transaction = conn.execute_batch("BEGIN").is_ok();
    if !in_transaction {
        tracing::warn!("failed to begin edge transaction — falling back to autocommit");
    }

    if let Err(e) =
        storage::edges::delete_edges_by_type(&conn, &["calls", "imports", "extends", "contains"])
    {
        tracing::warn!("failed to clear structural edges: {}", e);
    }

    let now = chrono::Utc::now().to_rfc3339();

    for edge in &edges {
        let db_edge = Edge {
            source_id: edge.source_id.clone(),
            target_id: edge.target_id.clone(),
            edge_type: edge.edge_type.to_string(),
            weight: 1.0,
            created_at: now.clone(),
        };
        if let Err(e) = storage::edges::insert_edge_skip_fk_violation(&conn, &db_edge) {
            tracing::debug!("skipped edge {}: {}", edge.edge_type, e);
        }
    }

    if in_transaction && let Err(e) = conn.execute_batch("COMMIT") {
        tracing::warn!("failed to commit edge transaction: {}", e);
    }
}
