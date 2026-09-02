//! Tree-sitter AST parsing and code entity + edge extraction.
//!
//! Parses source files with tree-sitter in a single pass and extracts
//! code entities (functions, classes, files, modules) with their
//! properties, plus raw structural edges (calls, imports, extends)
//! with name-based references that are resolved to UUIDs by the
//! code_graph module after all files are parsed.
//!
//! Supports Rust and Python via per-language extractors.

mod python;
mod raw_edges;
#[cfg(test)]
mod tests;

use std::path::Path;

use serde::Serialize;
use serde_json::json;
use tree_sitter::{Node, Parser};
use tree_sitter_language::LanguageFn;

/// A code entity extracted from source, ready for DB sync.
#[derive(Debug, Clone, Serialize)]
pub struct CodeEntity {
    /// Entity type: "function", "class", "file", "module".
    pub entity_type: &'static str,
    pub title: String,
    pub content: String,
    pub properties: serde_json::Value,
}

/// A structural edge with unresolved name references.
///
/// Produced during the single-pass AST walk alongside entities.
/// The `code_graph` module resolves `target_name` to a UUID using
/// the global name→UUID map built from all parsed entities.
#[derive(Debug, Clone)]
pub struct RawEdge {
    /// Entity type of the source: "function", "file", "class".
    pub source_type: &'static str,
    /// Qualified name of the source entity (for UUID lookup).
    pub source_name: String,
    /// Unresolved name of the target entity.
    pub target_name: String,
    /// Edge type: "calls", "imports", "extends".
    pub edge_type: &'static str,
}

/// Supported source languages for code indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            _ => None,
        }
    }

    pub fn tree_sitter_language(&self) -> LanguageFn {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE,
            Self::Python => tree_sitter_python::LANGUAGE,
        }
    }
}

/// Parse a source file and extract all code entities.
///
/// Returns a list of `CodeEntity` values: one `file` entity, zero or
/// more `module` entities, and zero or more `function`/`class` entities.
pub fn extract_entities(file_path: &Path, source: &str, language: Language) -> Vec<CodeEntity> {
    let mut parser = Parser::new();
    if parser
        .set_language(&language.tree_sitter_language().into())
        .is_err()
    {
        tracing::warn!(
            "failed to set tree-sitter language for {}",
            file_path.display()
        );
        return Vec::new();
    }

    let tree = match parser.parse(source.as_bytes(), None) {
        Some(t) => t,
        None => {
            tracing::warn!("failed to parse {}", file_path.display());
            return Vec::new();
        }
    };

    let path_str = file_path.to_string_lossy().to_string();
    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut entities = Vec::new();

    // File entity — always present
    entities.push(CodeEntity {
        entity_type: "file",
        title: file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone()),
        content: source.to_string(),
        properties: json!({
            "file_path": path_str,
            "language": language.as_str(),
            "line_count": source.lines().count(),
        }),
    });

    match language {
        Language::Rust => extract_rust(&root, source_bytes, &path_str, &mut entities),
        Language::Python => python::extract_python(&root, source_bytes, &path_str, &mut entities),
    }

    entities
}

/// Parse a source file once and extract both entities and raw edges.
///
/// This is the single-pass version that avoids re-parsing for edge
/// extraction. Returns the same entities as `extract_entities` plus
/// `RawEdge` values with unresolved name references.
pub fn extract_all(
    file_path: &Path,
    source: &str,
    language: Language,
) -> (Vec<CodeEntity>, Vec<RawEdge>) {
    let mut parser = Parser::new();
    if parser
        .set_language(&language.tree_sitter_language().into())
        .is_err()
    {
        tracing::warn!(
            "failed to set tree-sitter language for {}",
            file_path.display()
        );
        return (Vec::new(), Vec::new());
    }

    let tree = match parser.parse(source.as_bytes(), None) {
        Some(t) => t,
        None => {
            tracing::warn!("failed to parse {}", file_path.display());
            return (Vec::new(), Vec::new());
        }
    };

    let path_str = file_path.to_string_lossy().to_string();
    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut entities = Vec::new();
    let mut edges = Vec::new();

    // File entity — always present
    entities.push(CodeEntity {
        entity_type: "file",
        title: file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone()),
        content: source.to_string(),
        properties: json!({
            "file_path": path_str,
            "language": language.as_str(),
            "line_count": source.lines().count(),
        }),
    });

    match language {
        Language::Rust => {
            extract_rust(&root, source_bytes, &path_str, &mut entities);
            raw_edges::extract_rust_raw_edges(&root, source_bytes, &path_str, &mut edges);
        }
        Language::Python => {
            python::extract_python(&root, source_bytes, &path_str, &mut entities);
            python::extract_python_raw_edges(&root, source_bytes, &path_str, &mut edges);
        }
    }

    (entities, edges)
}

pub(super) fn node_text(node: &Node, source: &[u8]) -> Option<String> {
    node.utf8_text(source).ok().map(|s| s.to_string())
}

// ── Rust extraction ───────────────────────────────────────────────

fn extract_rust(root: &Node, source: &[u8], file_path: &str, entities: &mut Vec<CodeEntity>) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(entity) = extract_rust_function(&child, source, file_path) {
                    entities.push(entity);
                }
            }
            "struct_item" | "enum_item" | "trait_item" => {
                if let Some(entity) = extract_rust_type(&child, source, file_path, child.kind()) {
                    entities.push(entity);
                }
            }
            "impl_item" => {
                if let Some(entity) = extract_rust_impl(&child, source, file_path) {
                    entities.push(entity);
                }
                extract_rust_impl_methods(&child, source, file_path, entities);
            }
            "mod_item" => {
                if let Some(entity) = extract_rust_module(&child, source, file_path) {
                    entities.push(entity);
                }
            }
            _ => {}
        }
    }
}

fn extract_rust_function(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;
    let signature = content.lines().next().unwrap_or("").to_string();

    Some(CodeEntity {
        entity_type: "function",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "rust",
            "signature": signature,
            "qualified_name": name,
        }),
    })
}

fn extract_rust_type(
    node: &Node,
    source: &[u8],
    file_path: &str,
    kind: &str,
) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    Some(CodeEntity {
        entity_type: "class",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "rust",
            "qualified_name": name,
            "kind": kind,
        }),
    })
}

fn extract_rust_impl(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let type_node = node.child_by_field_name("type")?;
    let type_name = node_text(&type_node, source)?;
    let trait_name = node
        .child_by_field_name("trait")
        .and_then(|n| node_text(&n, source));
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    let title = match &trait_name {
        Some(trait_) => format!("impl {trait_} for {type_name}"),
        None => format!("impl {type_name}"),
    };

    // Qualified name includes "impl" prefix to distinguish from the
    // struct/class entity with the same type name.
    let qualified_name = match &trait_name {
        Some(trait_) => format!("impl {trait_} for {type_name}"),
        None => format!("impl {type_name}"),
    };

    Some(CodeEntity {
        entity_type: "class",
        title,
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "rust",
            "qualified_name": qualified_name,
            "kind": "impl",
            "trait": trait_name,
        }),
    })
}

fn extract_rust_impl_methods(
    impl_node: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let body = match impl_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "function_item"
            && let Some(mut entity) = extract_rust_function(&child, source, file_path)
        {
            if let Some(type_node) = impl_node.child_by_field_name("type")
                && let Some(type_name) = node_text(&type_node, source)
                && let Some(props) = entity.properties.as_object_mut()
            {
                let method_name = props
                    .get("qualified_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                props.insert(
                    "qualified_name".to_string(),
                    json!(format!("{type_name}::{method_name}")),
                );
            }
            entities.push(entity);
        }
    }
}

fn extract_rust_module(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let content = node_text(node, source)?;

    Some(CodeEntity {
        entity_type: "module",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "language": "rust",
            "module_path": name,
        }),
    })
}

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
