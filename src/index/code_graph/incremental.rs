//! Incremental edge sync — only re-extracts edges for changed files,
//! preserving edges from unchanged files.

use std::collections::HashMap;

use crate::index::code_graph::{CodeEdge, build_contains_edges, resolve_edges};
use crate::index::sync::code_entity_uuid;
use crate::index::tree_sitter::{Language, RawEdge};
use crate::storage;
use crate::storage::edges::Edge;

/// Incremental edge sync — only deletes and re-inserts edges for
/// entities in the changed files. Preserves edges from unchanged files.
///
/// Must be called after `sync_code_entities_incremental` so that
/// entities exist in the DB.
///
/// The name → UUID map is built from all code entities in the DB (not
/// just changed files) so that cross-file call targets in unchanged
/// files can be resolved.
pub fn sync_code_edges_incremental(
    storage: &storage::Storage,
    source_files: &[(std::path::PathBuf, String, Language)],
) {
    // Build name → UUID map from ALL code entities in the DB.
    let conn = storage.conn();
    let mut name_to_uuid: HashMap<String, String> = HashMap::new();
    let code_types = ["function", "class", "file", "module"];
    for entity_type in &code_types {
        if let Ok(entities) =
            storage::query::get_entities_by_type(&conn, entity_type, None, 100_000)
        {
            for entity in entities {
                // File entities: the title is just the filename (e.g.
                // "main.rs"), but raw import edges use the full relative
                // path (e.g. "src/main.rs") as source_name. Insert the
                // file_path column as a key so import edges can resolve,
                // matching build_name_map's use of file_path_str.
                if entity.r#type == "file"
                    && let Some(ref fp) = entity.file_path
                {
                    name_to_uuid.insert(fp.clone(), entity.id.clone());
                }
                if let Some(ref title) = entity.title {
                    name_to_uuid.insert(title.clone(), entity.id.clone());
                    if let Some(simple) = title.rsplit("::").next() {
                        name_to_uuid
                            .entry(simple.to_string())
                            .or_insert_with(|| entity.id.clone());
                    }
                    if let Some(simple) = title.rsplit('.').next() {
                        name_to_uuid
                            .entry(simple.to_string())
                            .or_insert_with(|| entity.id.clone());
                    }
                }
            }
        }
    }
    drop(conn);

    // Single-pass parse of changed files: extract entities + raw edges.
    let mut changed_entity_ids: Vec<String> = Vec::new();
    let mut file_to_children: HashMap<String, Vec<String>> = HashMap::new();
    let mut all_raw_edges: Vec<(String, Vec<RawEdge>)> = Vec::new();

    for (rel_path, source, language) in source_files {
        // Pass rel_path (not abs_path) to extract_all so that raw edge
        // source_name matches the DB's file_path column. extract_all
        // doesn't read the file — source is passed in — so the path is
        // only used for string representation in entities and edges.
        let (entities, raw_edges) =
            crate::index::tree_sitter::extract_all(rel_path, source, *language);

        let file_path_str = rel_path.to_string_lossy().to_string();
        for ce in &entities {
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

            let id = code_entity_uuid(&file_path_str, ce.entity_type, &qualified_name);
            changed_entity_ids.push(id.clone());

            if ce.entity_type == "function" || ce.entity_type == "class" {
                file_to_children
                    .entry(file_path_str.clone())
                    .or_default()
                    .push(id);
            }
        }

        all_raw_edges.push((file_path_str, raw_edges));
    }

    // Resolve raw edges using the global name map.
    let mut edges: Vec<CodeEdge> = Vec::new();
    for (_, raw_edges) in &all_raw_edges {
        edges.extend(resolve_edges(raw_edges, &name_to_uuid));
    }

    // Build `contains` edges from file entities to their functions/classes.
    edges.extend(build_contains_edges(&file_to_children));

    // Delete only edges sourced from changed entities, then re-insert.
    let conn = storage.conn();
    if let Err(e) = storage::edges::delete_structural_edges_by_sources(&conn, &changed_entity_ids) {
        tracing::warn!("failed to clear structural edges for changed files: {}", e);
    }

    let now = chrono::Utc::now().to_rfc3339();

    if let Err(e) = conn.execute_batch("BEGIN") {
        tracing::warn!(
            "failed to begin edge transaction: {} — falling back to autocommit",
            e
        );
    }
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
    if let Err(e) = conn.execute_batch("COMMIT") {
        tracing::warn!("failed to commit edge transaction: {}", e);
    }
}
