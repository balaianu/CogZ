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
///
/// Qualified names are unique by construction (they include the file
/// path or module path), so they are stored directly. Simple names
/// may collide across files (e.g. `parse` in two different modules);
/// for those, we store all candidates and skip resolution when the
/// name is ambiguous rather than picking an arbitrary target.
#[allow(clippy::type_complexity)]
pub fn build_name_map(
    entities_by_file: &[(String, Vec<CodeEntity>)],
) -> (
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
) {
    // Qualified name → all candidate UUIDs. One-to-many to detect
    // ambiguity: duplicate top-level names (e.g. two `helper`
    // functions in different JS files) produce multiple candidates.
    // Direct lookup only resolves when there's exactly one candidate.
    let mut name_to_uuid: HashMap<String, Vec<String>> = HashMap::new();
    let mut file_to_children: HashMap<String, Vec<String>> = HashMap::new();
    // Simple name → all candidate UUIDs. Used for fallback resolution.
    let mut simple_name_candidates: HashMap<String, Vec<String>> = HashMap::new();

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

            name_to_uuid
                .entry(qualified_name.clone())
                .or_default()
                .push(id.clone());
            // Collect simple-name candidates for ambiguity detection.
            // Use a set to avoid double-counting when the qualified name
            // has no separator (both rsplit("::") and rsplit('.') return
            // the whole string, producing the same simple name).
            let mut simples = std::collections::HashSet::new();
            if let Some(simple) = qualified_name.rsplit("::").next() {
                simples.insert(simple.to_string());
            }
            // Python uses dot separators in qualified names.
            if let Some(simple) = qualified_name.rsplit('.').next() {
                simples.insert(simple.to_string());
            }
            for simple in simples {
                simple_name_candidates
                    .entry(simple)
                    .or_default()
                    .push(id.clone());
            }

            if ce.entity_type == "function" || ce.entity_type == "class" {
                file_to_children
                    .entry(file_path_str.clone())
                    .or_default()
                    .push(id);
            }
        }
    }

    (name_to_uuid, file_to_children, simple_name_candidates)
}

/// Resolve a name to a single UUID via the one-to-many map.
/// Returns None when the name is ambiguous (multiple candidates)
/// or not found.
fn resolve_one<'a>(name: &str, name_to_uuid: &'a HashMap<String, Vec<String>>) -> Option<&'a str> {
    match name_to_uuid.get(name) {
        Some(candidates) if candidates.len() == 1 => Some(&candidates[0]),
        _ => None,
    }
}

/// Resolve raw edges to UUID-based CodeEdges using the name map.
///
/// Tries the full target name first, then the last segment (after
/// `::` or `.`) as a fallback. Both direct and fallback lookups
/// skip ambiguous names (multiple candidates) — picking one would
/// create an arbitrary, non-rebuildable edge. Unresolved edges are
/// silently dropped.
pub fn resolve_edges(
    raw_edges: &[RawEdge],
    name_to_uuid: &HashMap<String, Vec<String>>,
    simple_name_candidates: &HashMap<String, Vec<String>>,
) -> Vec<CodeEdge> {
    let mut edges = Vec::new();
    for raw in raw_edges {
        let source_id = resolve_one(&raw.source_name, name_to_uuid);
        let target_id = resolve_one(&raw.target_name, name_to_uuid).or_else(|| {
            // Try simple-name fallback only if unambiguous.
            let simple = raw
                .target_name
                .rsplit("::")
                .next()
                .or_else(|| raw.target_name.rsplit('.').next())?;
            match simple_name_candidates.get(simple) {
                Some(candidates) if candidates.len() == 1 => Some(&candidates[0]),
                _ => None,
            }
        });

        if let (Some(src), Some(tgt)) = (source_id, target_id) {
            edges.push(CodeEdge {
                source_id: src.to_string(),
                target_id: tgt.to_string(),
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
    let (name_to_uuid, file_to_children, simple_name_candidates) = build_name_map(entities_by_file);

    // Phase 2: resolve raw edges to UUID-based edges.
    let mut edges: Vec<CodeEdge> = Vec::new();
    for (_, raw_edges) in raw_edges_by_file {
        edges.extend(resolve_edges(
            raw_edges,
            &name_to_uuid,
            &simple_name_candidates,
        ));
    }

    // Phase 2b: build `contains` edges.
    edges.extend(build_contains_edges(&file_to_children));

    // Phase 3: sync edges to DB. The DELETE and INSERTs are in a
    // single transaction so a COMMIT failure rolls back both —
    // otherwise the DELETE would persist (autocommit) while the
    // INSERTs roll back, silently destroying the graph.
    // Use unchecked_transaction so the guard rolls back on drop
    // if any error occurs before commit.
    let conn = storage.conn();

    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!("failed to begin edge transaction: {}", e);
            return;
        }
    };

    if let Err(e) =
        storage::edges::delete_edges_by_type(&tx, &["calls", "imports", "extends", "contains"])
    {
        tracing::warn!("failed to clear structural edges: {}", e);
        // tx drops here → automatic ROLLBACK.
        return;
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
        if let Err(e) = storage::edges::insert_edge_skip_fk_violation(&tx, &db_edge) {
            tracing::debug!("skipped edge {}: {}", edge.edge_type, e);
        }
    }

    if let Err(e) = tx.commit() {
        tracing::warn!("failed to commit edge transaction: {}", e);
    }
}
