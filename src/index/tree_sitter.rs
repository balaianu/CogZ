//! Tree-sitter AST parsing and code entity extraction.
//!
//! Parses source files with tree-sitter and extracts code entities
//! (functions, classes, files, modules) with their properties.
//! Supports Rust and Python via per-language extractors.

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
    /// Human-readable title (function name, class name, file path, module path).
    pub title: String,
    /// Source code text of the entity.
    pub content: String,
    /// JSON properties stored in the DB.
    pub properties: serde_json::Value,
}

/// Supported source languages.
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

    pub fn from_str(s: &str) -> Option<Self> {
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
        tracing::warn!("failed to set tree-sitter language for {}", file_path.display());
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
        Language::Python => extract_python(&root, source_bytes, &path_str, &mut entities),
    }

    entities
}

// ── Rust extraction ───────────────────────────────────────────────

fn extract_rust(
    root: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
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
                // Also extract methods inside impl blocks as functions
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

fn extract_rust_function(
    node: &Node,
    source: &[u8],
    file_path: &str,
) -> Option<CodeEntity> {
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

fn extract_rust_impl(
    node: &Node,
    source: &[u8],
    file_path: &str,
) -> Option<CodeEntity> {
    let type_node = node.child_by_field_name("type")?;
    let type_name = node_text(&type_node, source)?;
    let trait_name = node.child_by_field_name("trait").and_then(|n| node_text(&n, source));
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
        if child.kind() == "function_item" {
            if let Some(mut entity) = extract_rust_function(&child, source, file_path) {
                // Prefix method name with the impl type for qualified_name
                if let Some(type_node) = impl_node.child_by_field_name("type") {
                    if let Some(type_name) = node_text(&type_node, source) {
                        if let Some(props) = entity.properties.as_object_mut() {
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
                    }
                }
                entities.push(entity);
            }
        }
    }
}

fn extract_rust_module(
    node: &Node,
    source: &[u8],
    file_path: &str,
) -> Option<CodeEntity> {
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

// ── Python extraction ─────────────────────────────────────────────

fn extract_python(
    root: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(entity) = extract_python_function(&child, source, file_path) {
                    entities.push(entity);
                }
            }
            "class_definition" => {
                if let Some(entity) = extract_python_class(&child, source, file_path) {
                    entities.push(entity);
                }
                // Extract methods inside the class
                extract_python_class_methods(&child, source, file_path, entities);
            }
            _ => {}
        }
    }
}

fn extract_python_function(
    node: &Node,
    source: &[u8],
    file_path: &str,
) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    // Build a signature from the `def` line(s) up to the `:`
    let signature = content
        .lines()
        .take_while(|l| !l.trim_start().starts_with('\"') && !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Some(CodeEntity {
        entity_type: "function",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "python",
            "signature": signature,
            "qualified_name": name,
        }),
    })
}

fn extract_python_class(
    node: &Node,
    source: &[u8],
    file_path: &str,
) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    // Extract superclasses for extends edges
    let superclasses: Vec<String> = node
        .child_by_field_name("superclasses")
        .and_then(|sc| {
            let mut cursor = sc.walk();
            Some(sc.named_children(&mut cursor).filter_map(|c| node_text(&c, source)).collect())
        })
        .unwrap_or_default();

    Some(CodeEntity {
        entity_type: "class",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "python",
            "qualified_name": name,
            "superclasses": superclasses,
        }),
    })
}

fn extract_python_class_methods(
    class_node: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let body = match class_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "function_definition" {
            if let Some(mut entity) = extract_python_function(&child, source, file_path) {
                // Prefix method name with the class name
                if let Some(name_node) = class_node.child_by_field_name("name") {
                    if let Some(class_name) = node_text(&name_node, source) {
                        if let Some(props) = entity.properties.as_object_mut() {
                            let method_name = props
                                .get("qualified_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            props.insert(
                                "qualified_name".to_string(),
                                json!(format!("{class_name}::{method_name}")),
                            );
                        }
                    }
                }
                entities.push(entity);
            }
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────

fn node_text<'a>(node: &Node, source: &'a [u8]) -> Option<String> {
    node.utf8_text(source).ok().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rust_function() {
        let code = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let entities = extract_entities(Path::new("src/math.rs"), code, Language::Rust);
        let funcs: Vec<_> = entities.iter().filter(|e| e.entity_type == "function").collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].title, "add");
        assert_eq!(funcs[0].properties["line_start"], 2);
        assert_eq!(funcs[0].properties["line_end"], 4);
        assert_eq!(funcs[0].properties["language"], "rust");
        assert!(funcs[0].properties["signature"].as_str().unwrap().contains("fn add"));
    }

    #[test]
    fn extract_rust_struct() {
        let code = r#"
pub struct Point {
    x: f64,
    y: f64,
}
"#;
        let entities = extract_entities(Path::new("src/point.rs"), code, Language::Rust);
        let classes: Vec<_> = entities.iter().filter(|e| e.entity_type == "class").collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].title, "Point");
        assert_eq!(classes[0].properties["kind"], "struct_item");
    }

    #[test]
    fn extract_rust_impl_with_methods() {
        let code = r#"
impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn distance(&self, other: &Point) -> f64 {
        0.0
    }
}
"#;
        let entities = extract_entities(Path::new("src/point.rs"), code, Language::Rust);
        let classes: Vec<_> = entities.iter().filter(|e| e.entity_type == "class").collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].title, "impl Point");

        let funcs: Vec<_> = entities.iter().filter(|e| e.entity_type == "function").collect();
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].properties["qualified_name"], "Point::new");
        assert_eq!(funcs[1].properties["qualified_name"], "Point::distance");
    }

    #[test]
    fn extract_rust_module() {
        let code = r#"
pub mod storage {
    pub fn foo() {}
}
"#;
        let entities = extract_entities(Path::new("src/lib.rs"), code, Language::Rust);
        let mods: Vec<_> = entities.iter().filter(|e| e.entity_type == "module").collect();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].title, "storage");
        assert_eq!(mods[0].properties["module_path"], "storage");
    }

    #[test]
    fn extract_rust_file_entity_always_present() {
        let code = "fn main() {}";
        let entities = extract_entities(Path::new("src/main.rs"), code, Language::Rust);
        let files: Vec<_> = entities.iter().filter(|e| e.entity_type == "file").collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].title, "main.rs");
        assert_eq!(files[0].properties["language"], "rust");
        assert_eq!(files[0].properties["line_count"], 1);
    }

    #[test]
    fn extract_python_function() {
        let code = r#"
def add(a, b):
    return a + b
"#;
        let entities = extract_entities(Path::new("math.py"), code, Language::Python);
        let funcs: Vec<_> = entities.iter().filter(|e| e.entity_type == "function").collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].title, "add");
        assert_eq!(funcs[0].properties["line_start"], 2);
        assert_eq!(funcs[0].properties["language"], "python");
    }

    #[test]
    fn extract_python_class_with_methods() {
        let code = r#"
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def distance(self, other):
        return 0.0
"#;
        let entities = extract_entities(Path::new("point.py"), code, Language::Python);
        let classes: Vec<_> = entities.iter().filter(|e| e.entity_type == "class").collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].title, "Point");

        let funcs: Vec<_> = entities.iter().filter(|e| e.entity_type == "function").collect();
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].properties["qualified_name"], "Point::__init__");
        assert_eq!(funcs[1].properties["qualified_name"], "Point::distance");
    }

    #[test]
    fn extract_python_class_with_superclass() {
        let code = r#"
class Dog(Animal):
    def bark(self):
        pass
"#;
        let entities = extract_entities(Path::new("dog.py"), code, Language::Python);
        let classes: Vec<_> = entities.iter().filter(|e| e.entity_type == "class").collect();
        assert_eq!(classes.len(), 1);
        let superclasses = classes[0].properties["superclasses"].as_array().unwrap();
        assert_eq!(superclasses.len(), 1);
        assert_eq!(superclasses[0], "Animal");
    }

    #[test]
    fn extract_python_file_entity_always_present() {
        let code = "x = 1";
        let entities = extract_entities(Path::new("script.py"), code, Language::Python);
        let files: Vec<_> = entities.iter().filter(|e| e.entity_type == "file").collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].title, "script.py");
        assert_eq!(files[0].properties["language"], "python");
    }

    #[test]
    fn empty_file_produces_only_file_entity() {
        let code = "";
        let entities = extract_entities(Path::new("empty.rs"), code, Language::Rust);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "file");
    }

    #[test]
    fn syntax_error_does_not_panic() {
        let code = "fn broken(";
        let entities = extract_entities(Path::new("bad.rs"), code, Language::Rust);
        // File entity is always present; broken function may or may not extract
        assert!(!entities.is_empty());
        assert_eq!(entities[0].entity_type, "file");
    }
}
