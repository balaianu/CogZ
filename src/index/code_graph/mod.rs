//! Structural edge extraction — calls, imports, extends.
//!
//! Walks the tree-sitter AST a second time after entity sync to
//! extract structural relationships between code entities. Edges are
//! matched by name to the deterministic UUIDs assigned during sync.

mod incremental;
pub(crate) mod python;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Parser};
use tree_sitter_language::LanguageFn;

use crate::index::sync::code_entity_uuid;
use crate::index::tree_sitter::Language;
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

/// Extract and sync structural edges for a set of source files.
///
/// Must be called after `sync_code_entities` so that all entities
/// exist in the DB for FK constraints.
///
/// **Full scan only.** This function deletes ALL structural edges
/// and rebuilds from the given source files. For incremental reindex,
/// use `sync_code_edges_incremental` instead.
pub fn sync_code_edges(
    storage: &storage::Storage,
    repo_root: &Path,
    source_files: &[(std::path::PathBuf, String, Language)],
) {
    // Build a name → UUID lookup for each file.
    // Key: (file_path, entity_type, qualified_name)
    // We also build a global name → UUID map for cross-file calls.
    let mut name_to_uuid: HashMap<String, String> = HashMap::new();

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

            // Map both the qualified name and the simple name.
            // For "Point::new", map both "Point::new" and "new".
            name_to_uuid.insert(qualified_name.clone(), id.clone());
            if let Some(simple) = qualified_name.rsplit("::").next() {
                name_to_uuid
                    .entry(simple.to_string())
                    .or_insert_with(|| id.clone());
            }
        }
    }

    // Phase 2: extract edges from AST.
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

    // Phase 3: sync edges to DB.
    let conn = storage.conn();

    // Clear existing structural edges before re-inserting. Edges are
    // fully derived from source code, so a delete+rebuild is correct
    // and prevents stale edges from accumulating when code changes.
    if let Err(e) = storage::edges::delete_edges_by_type(&conn, &["calls", "imports", "extends"]) {
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
        // Use skip_fk_violation in case target entity doesn't exist
        // (e.g. external function not indexed).
        if let Err(e) = storage::edges::insert_edge_skip_fk_violation(&conn, &db_edge) {
            tracing::debug!("skipped edge {}: {}", edge.edge_type, e);
        }
    }
}

// ── Rust edge extraction ──────────────────────────────────────────

pub(crate) fn extract_rust_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    // Get the file entity UUID for imports edges.
    let file_uuid = code_entity_uuid(file_path, "file", file_path);

    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "use_declaration" => {
                extract_rust_import(&node, source, &file_uuid, name_to_uuid, edges);
            }
            "function_item" => {
                let func_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                let source_id = code_entity_uuid(file_path, "function", &func_name);
                extract_rust_calls_in_node(&node, source, &source_id, name_to_uuid, edges);
            }
            "impl_item" => {
                extract_rust_extends_in_impl(&node, source, name_to_uuid, edges);
                // Also extract calls from methods inside impl.
                let mut impl_cursor = node.walk();
                for child in node.named_children(&mut impl_cursor) {
                    if child.kind() == "function_item" {
                        let method_name = child
                            .child_by_field_name("name")
                            .and_then(|n| node_text(&n, source))
                            .unwrap_or_default();
                        // For methods, the qualified_name is Type::method
                        let type_name = node
                            .child_by_field_name("type")
                            .and_then(|n| node_text(&n, source))
                            .unwrap_or_default();
                        let qualified = format!("{type_name}::{method_name}");
                        let source_id = code_entity_uuid(file_path, "function", &qualified);
                        extract_rust_calls_in_node(&child, source, &source_id, name_to_uuid, edges);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_rust_import(
    node: &Node,
    source: &[u8],
    file_uuid: &str,
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    // The `argument` field contains the imported path.
    if let Some(arg) = node.child_by_field_name("argument") {
        let import_text = node_text(&arg, source).unwrap_or_default();
        // Try to match the full path or the last segment.
        if let Some(target_id) = name_to_uuid.get(&import_text) {
            edges.push(CodeEdge {
                source_id: file_uuid.to_string(),
                target_id: target_id.clone(),
                edge_type: "imports",
            });
        } else if let Some(last) = import_text.rsplit("::").next()
            && let Some(target_id) = name_to_uuid.get(last)
        {
            edges.push(CodeEdge {
                source_id: file_uuid.to_string(),
                target_id: target_id.clone(),
                edge_type: "imports",
            });
        }
    }
}

fn extract_rust_calls_in_node(
    node: &Node,
    source: &[u8],
    source_id: &str,
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    // Walk all descendants looking for call_expression nodes.
    let mut f = |desc: &Node| {
        if desc.kind() == "call_expression"
            && let Some(func_node) = desc.child_by_field_name("function")
        {
            let call_name = node_text(&func_node, source).unwrap_or_default();
            // Try full path, then last segment.
            if let Some(target_id) = name_to_uuid.get(&call_name) {
                edges.push(CodeEdge {
                    source_id: source_id.to_string(),
                    target_id: target_id.clone(),
                    edge_type: "calls",
                });
            } else if let Some(last) = call_name.rsplit("::").next()
                && let Some(target_id) = name_to_uuid.get(last)
            {
                edges.push(CodeEdge {
                    source_id: source_id.to_string(),
                    target_id: target_id.clone(),
                    edge_type: "calls",
                });
            }
        }
        true
    };
    walk_descendants(node, &mut f);
}

fn extract_rust_extends_in_impl(
    node: &Node,
    source: &[u8],
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    // impl Trait for Type → extends edge from Type to Trait
    let type_node = node.child_by_field_name("type");
    let trait_node = node.child_by_field_name("trait");

    if let (Some(type_n), Some(trait_n)) = (type_node, trait_node) {
        let type_name = node_text(&type_n, source).unwrap_or_default();
        let trait_name = node_text(&trait_n, source).unwrap_or_default();

        let type_id = name_to_uuid.get(&type_name);
        let trait_id = name_to_uuid.get(&trait_name);

        if let (Some(source_id), Some(target_id)) = (type_id, trait_id) {
            edges.push(CodeEdge {
                source_id: source_id.clone(),
                target_id: target_id.clone(),
                edge_type: "extends",
            });
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────

pub(super) fn node_text(node: &Node, source: &[u8]) -> Option<String> {
    node.utf8_text(source).ok().map(|s| s.to_string())
}

/// Recursively walk all named descendants of a node, calling `f` on
/// each. If `f` returns false, the walk stops.
pub(super) fn walk_descendants<F>(node: &Node, f: &mut F)
where
    F: FnMut(&Node) -> bool,
{
    let num_named = node.named_child_count();
    for i in 0..num_named {
        if let Some(child) = node.named_child(i) {
            if !f(&child) {
                return;
            }
            walk_descendants(&child, f);
        }
    }
}
