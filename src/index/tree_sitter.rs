//! Tree-sitter AST parsing and code entity + edge extraction.
//!
//! Parses source files with tree-sitter in a single pass and extracts
//! code entities (functions, classes, files, modules) with their
//! properties, plus raw structural edges (calls, imports, extends)
//! with name-based references that are resolved to UUIDs by the
//! code_graph module after all files are parsed.
//!
//! Supports Rust, Python, Go, JavaScript, TypeScript, and Bash via
//! per-language extractors.

mod bash;
mod go;
mod javascript;
mod python;
mod raw_edges;
mod rust;
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
    Go,
    JavaScript,
    TypeScript,
    Tsx,
    Bash,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Bash => "bash",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "go" => Some(Self::Go),
            "javascript" => Some(Self::JavaScript),
            "typescript" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "bash" => Some(Self::Bash),
            _ => None,
        }
    }

    pub fn tree_sitter_language(&self) -> LanguageFn {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE,
            Self::Python => tree_sitter_python::LANGUAGE,
            Self::Go => tree_sitter_go::LANGUAGE,
            Self::JavaScript => tree_sitter_javascript::LANGUAGE,
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX,
            Self::Bash => tree_sitter_bash::LANGUAGE,
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

    dispatch_extract(&language, &root, source_bytes, &path_str, &mut entities);

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

    dispatch_extract(&language, &root, source_bytes, &path_str, &mut entities);
    dispatch_raw_edges(&language, &root, source_bytes, &path_str, &mut edges);

    (entities, edges)
}

/// Dispatch entity extraction to the appropriate language module.
fn dispatch_extract(
    language: &Language,
    root: &Node,
    source: &[u8],
    path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    match language {
        Language::Rust => rust::extract_rust(root, source, path, entities),
        Language::Python => python::extract_python(root, source, path, entities),
        Language::Go => go::extract_go(root, source, path, entities),
        Language::JavaScript => {
            javascript::extract_javascript(root, source, path, "javascript", entities)
        }
        Language::TypeScript => {
            javascript::extract_javascript(root, source, path, "typescript", entities)
        }
        Language::Tsx => javascript::extract_javascript(root, source, path, "tsx", entities),
        Language::Bash => bash::extract_bash(root, source, path, entities),
    }
}

/// Dispatch raw edge extraction to the appropriate language module.
fn dispatch_raw_edges(
    language: &Language,
    root: &Node,
    source: &[u8],
    path: &str,
    edges: &mut Vec<RawEdge>,
) {
    match language {
        Language::Rust => raw_edges::extract_rust_raw_edges(root, source, path, edges),
        Language::Python => python::extract_python_raw_edges(root, source, path, edges),
        Language::Go => go::extract_go_raw_edges(root, source, path, edges),
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            javascript::extract_javascript_raw_edges(root, source, path, edges)
        }
        Language::Bash => bash::extract_bash_raw_edges(root, source, path, edges),
    }
}

pub(super) fn node_text(node: &Node, source: &[u8]) -> Option<String> {
    node.utf8_text(source).ok().map(|s| s.to_string())
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
