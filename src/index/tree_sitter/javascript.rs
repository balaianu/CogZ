//! JavaScript/TypeScript AST extraction — functions, classes, imports.

use serde_json::json;
use tree_sitter::Node;

#[path = "js_ts_types.rs"]
mod js_ts_types;

#[path = "js_edges.rs"]
mod js_edges;

use js_edges::{collect_js_calls, collect_js_imports, collect_js_variable_calls};
use js_ts_types::{extract_ts_enum, extract_ts_interface};

use super::{CodeEntity, RawEdge, node_text, walk_descendants};

pub(super) fn extract_javascript(
    root: &Node,
    source: &[u8],
    file_path: &str,
    language: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(entity) = extract_js_function(&child, source, file_path, language) {
                    entities.push(entity);
                }
            }
            "class_declaration" => {
                if let Some(entity) = extract_js_class(&child, source, file_path, language) {
                    entities.push(entity);
                }
                extract_js_class_methods(&child, source, file_path, language, entities);
            }
            "variable_declaration" | "lexical_declaration" => {
                extract_js_variable_declarations(&child, source, file_path, language, entities);
            }
            "export_statement" => {
                // export function/class — recurse into the inner declaration
                let mut exp_cursor = child.walk();
                for inner in child.named_children(&mut exp_cursor) {
                    match inner.kind() {
                        "function_declaration" => {
                            if let Some(entity) =
                                extract_js_function(&inner, source, file_path, language)
                            {
                                entities.push(entity);
                            }
                        }
                        "class_declaration" => {
                            if let Some(entity) =
                                extract_js_class(&inner, source, file_path, language)
                            {
                                entities.push(entity);
                            }
                            extract_js_class_methods(&inner, source, file_path, language, entities);
                        }
                        "variable_declaration" | "lexical_declaration" => {
                            extract_js_variable_declarations(
                                &inner, source, file_path, language, entities,
                            );
                        }
                        _ => {}
                    }
                }
            }
            // TypeScript-specific
            "interface_declaration" if language == "typescript" || language == "tsx" => {
                if let Some(entity) = extract_ts_interface(&child, source, file_path, language) {
                    entities.push(entity);
                }
            }
            "enum_declaration" if language == "typescript" || language == "tsx" => {
                if let Some(entity) = extract_ts_enum(&child, source, file_path, language) {
                    entities.push(entity);
                }
            }
            _ => {}
        }
    }
}

fn extract_js_function(
    node: &Node,
    source: &[u8],
    file_path: &str,
    language: &str,
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
            "language": language,
            "signature": signature,
            "qualified_name": name,
        }),
    })
}

fn extract_js_class(
    node: &Node,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    let superclass = node
        .child_by_field_name("superclass")
        .or_else(|| {
            // tree-sitter-javascript uses class_heritage, not superclass field
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|c| c.kind() == "class_heritage")
        })
        .and_then(|sc| sc.named_child(0).and_then(|v| node_text(&v, source)));

    Some(CodeEntity {
        entity_type: "class",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": language,
            "qualified_name": name,
            "superclass": superclass,
        }),
    })
}

fn extract_js_class_methods(
    class_node: &Node,
    source: &[u8],
    file_path: &str,
    language: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let body = match class_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        let (method_name, is_static) = match child.kind() {
            "method_definition" => {
                let name_node = match child.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let name = match node_text(&name_node, source) {
                    Some(n) => n,
                    None => continue,
                };
                let is_static = child.child_by_field_name("static").is_some();
                (name, is_static)
            }
            _ => continue,
        };

        let line_start = child.start_position().row + 1;
        let line_end = child.end_position().row + 1;
        let content = node_text(&child, source).unwrap_or_default();
        let signature = content.lines().next().unwrap_or("").to_string();

        if let Some(class_name_node) = class_node.child_by_field_name("name")
            && let Some(class_name) = node_text(&class_name_node, source)
        {
            entities.push(CodeEntity {
                entity_type: "function",
                title: format!("{}::{}", class_name, method_name),
                content,
                properties: json!({
                    "file_path": file_path,
                    "line_start": line_start,
                    "line_end": line_end,
                    "language": language,
                    "signature": signature,
                    "qualified_name": format!("{class_name}::{method_name}"),
                    "is_static": is_static,
                }),
            });
        }
    }
}

fn extract_js_variable_declarations(
    node: &Node,
    source: &[u8],
    file_path: &str,
    language: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let mut cursor = node.walk();
    for decl in node.named_children(&mut cursor) {
        if decl.kind() != "variable_declarator" {
            continue;
        }
        let name_node = match decl.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let name = match node_text(&name_node, source) {
            Some(n) => n,
            None => continue,
        };
        // Only extract if the value is a function (arrow or function expression)
        let value_node = decl.child_by_field_name("value");
        let is_function = value_node
            .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression"));
        if !is_function {
            continue;
        }
        let line_start = decl.start_position().row + 1;
        let line_end = decl.end_position().row + 1;
        let content = node_text(&decl, source).unwrap_or_default();
        let signature = content.lines().next().unwrap_or("").to_string();

        entities.push(CodeEntity {
            entity_type: "function",
            title: name.clone(),
            content,
            properties: json!({
                "file_path": file_path,
                "line_start": line_start,
                "line_end": line_end,
                "language": language,
                "signature": signature,
                "qualified_name": name,
                "kind": "arrow_function",
            }),
        });
    }
}

// ── JS/TS raw edge extraction ──────────────────────────────────────

pub(super) fn extract_javascript_raw_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "import_statement" => {
                collect_js_imports(&node, source, file_path, edges);
            }
            "function_declaration" => {
                let func_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                if !func_name.is_empty() {
                    collect_js_calls(&node, source, "function", &func_name, edges);
                }
            }
            "variable_declaration" | "lexical_declaration" => {
                collect_js_variable_calls(&node, source, edges);
            }
            "class_declaration" => {
                let class_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                // extends: class Foo extends Bar
                let sc_node = node.child_by_field_name("superclass").or_else(|| {
                    let mut c = node.walk();
                    node.named_children(&mut c)
                        .find(|ch| ch.kind() == "class_heritage")
                });
                if let Some(sc_node) = sc_node
                    && let Some(sc_inner) = sc_node.named_child(0)
                {
                    let sc_name = node_text(&sc_inner, source).unwrap_or_default();
                    if !sc_name.is_empty() && !class_name.is_empty() {
                        edges.push(RawEdge {
                            source_type: "class",
                            source_name: class_name.clone(),
                            target_name: sc_name,
                            edge_type: "extends",
                        });
                    }
                }
                // calls from methods
                let mut class_cursor = node.walk();
                for child in node.named_children(&mut class_cursor) {
                    if child.kind() == "method_definition" {
                        let method_name = child
                            .child_by_field_name("name")
                            .and_then(|n| node_text(&n, source))
                            .unwrap_or_default();
                        if !method_name.is_empty() && !class_name.is_empty() {
                            let qualified = format!("{class_name}::{method_name}");
                            collect_js_calls(&child, source, "function", &qualified, edges);
                        }
                    }
                }
            }
            "export_statement" => {
                let mut exp_cursor = node.walk();
                for inner in node.named_children(&mut exp_cursor) {
                    match inner.kind() {
                        "function_declaration" => {
                            let func_name = inner
                                .child_by_field_name("name")
                                .and_then(|n| node_text(&n, source))
                                .unwrap_or_default();
                            if !func_name.is_empty() {
                                collect_js_calls(&inner, source, "function", &func_name, edges);
                            }
                        }
                        "class_declaration" => {
                            let class_name = inner
                                .child_by_field_name("name")
                                .and_then(|n| node_text(&n, source))
                                .unwrap_or_default();
                            let sc_node = inner.child_by_field_name("superclass").or_else(|| {
                                let mut c = inner.walk();
                                inner
                                    .named_children(&mut c)
                                    .find(|ch| ch.kind() == "class_heritage")
                            });
                            if let Some(sc_node) = sc_node
                                && let Some(sc_inner) = sc_node.named_child(0)
                            {
                                let sc_name = node_text(&sc_inner, source).unwrap_or_default();
                                if !sc_name.is_empty() && !class_name.is_empty() {
                                    edges.push(RawEdge {
                                        source_type: "class",
                                        source_name: class_name.clone(),
                                        target_name: sc_name,
                                        edge_type: "extends",
                                    });
                                }
                            }
                            let mut cls_cursor = inner.walk();
                            for m in inner.named_children(&mut cls_cursor) {
                                if m.kind() == "method_definition" {
                                    let method_name = m
                                        .child_by_field_name("name")
                                        .and_then(|n| node_text(&n, source))
                                        .unwrap_or_default();
                                    if !method_name.is_empty() && !class_name.is_empty() {
                                        let qualified = format!("{class_name}::{method_name}");
                                        collect_js_calls(&m, source, "function", &qualified, edges);
                                    }
                                }
                            }
                        }
                        "import_statement" => {
                            collect_js_imports(&inner, source, file_path, edges);
                        }
                        "variable_declaration" | "lexical_declaration" => {
                            collect_js_variable_calls(&inner, source, edges);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}
