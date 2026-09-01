//! Incremental edge sync — only re-extracts edges for changed files,
//! preserving edges from unchanged files.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::Parser;
use tree_sitter_language::LanguageFn;

use crate::index::code_graph::{CodeEdge, extract_rust_edges, python};
use crate::index::sync::code_entity_uuid;
use crate::index::tree_sitter::Language;
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
    repo_root: &Path,
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
                if let Some(ref title) = entity.title {
                    name_to_uuid.insert(title.clone(), entity.id.clone());
                    if let Some(simple) = title.rsplit("::").next() {
                        name_to_uuid
                            .entry(simple.to_string())
                            .or_insert_with(|| entity.id.clone());
                    }
                }
            }
        }
    }
    drop(conn);

    // Collect changed entity IDs for targeted edge deletion.
    // Also build file → children for `contains` edges.
    let mut changed_entity_ids: Vec<String> = Vec::new();
    let mut file_to_children: HashMap<String, Vec<String>> = HashMap::new();
    for (rel_path, source, language) in source_files {
        let abs_path = repo_root.join(rel_path);
        let entities = crate::index::tree_sitter::extract_entities(&abs_path, source, *language);

        let file_path_str = rel_path.to_string_lossy().to_string();
        for ce in entities {
            let qualified_name = ce
                .properties
                .get("qualified_name")
                .and_then(|v| v.as_str())
                .or_else(|| ce.properties.get("module_path").and_then(|v| v.as_str()))
                .unwrap_or(&ce.title)
                .to_string();

            let id = code_entity_uuid(&file_path_str, ce.entity_type, &qualified_name);
            changed_entity_ids.push(id.clone());

            if ce.entity_type == "function" || ce.entity_type == "class" {
                file_to_children
                    .entry(file_path_str.clone())
                    .or_default()
                    .push(id);
            }
        }
    }

    // Extract edges from changed files.
    let mut edges: Vec<CodeEdge> = Vec::new();
    for (rel_path, source, language) in source_files {
        let file_path_str = rel_path.to_string_lossy().to_string();
        let mut parser = Parser::new();
        let lang_fn: LanguageFn = language.tree_sitter_language();
        if parser.set_language(&lang_fn.into()).is_err() {
            continue;
        }
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => continue,
        };
        let root = tree.root_node();
        let source_bytes = source.as_bytes();

        match language {
            Language::Rust => extract_rust_edges(
                &root,
                source_bytes,
                &file_path_str,
                &name_to_uuid,
                &mut edges,
            ),
            Language::Python => python::extract_python_edges(
                &root,
                source_bytes,
                &file_path_str,
                &name_to_uuid,
                &mut edges,
            ),
        }
    }

    // Build `contains` edges from file entities to their functions/classes.
    for (file_path, children) in &file_to_children {
        let file_uuid = code_entity_uuid(file_path, "file", file_path);
        for child_id in children {
            edges.push(CodeEdge {
                source_id: file_uuid.clone(),
                target_id: child_id.clone(),
                edge_type: "contains",
            });
        }
    }

    // Delete only edges sourced from changed entities, then re-insert.
    let conn = storage.conn();
    if let Err(e) = storage::edges::delete_structural_edges_by_sources(&conn, &changed_entity_ids) {
        tracing::warn!("failed to clear structural edges for changed files: {}", e);
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
}
