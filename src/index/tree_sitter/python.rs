//! Python AST extraction — functions, classes, files, raw edges.

use serde_json::json;
use tree_sitter::Node;

use super::{CodeEntity, RawEdge, node_text, walk_descendants};

pub(super) fn extract_python(
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
                extract_python_class_methods(&child, source, file_path, entities);
            }
            _ => {}
        }
    }
}

fn extract_python_function(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    let signature = content
        .lines()
        .take_while(|l| !l.trim_start().starts_with('"') && !l.is_empty())
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

fn extract_python_class(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    let superclasses: Vec<String> = node
        .child_by_field_name("superclasses")
        .map(|sc| {
            let mut cursor = sc.walk();
            sc.named_children(&mut cursor)
                .filter_map(|c| node_text(&c, source))
                .collect()
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
        if child.kind() == "function_definition"
            && let Some(mut entity) = extract_python_function(&child, source, file_path)
        {
            if let Some(name_node) = class_node.child_by_field_name("name")
                && let Some(class_name) = node_text(&name_node, source)
                && let Some(props) = entity.properties.as_object_mut()
            {
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
            entities.push(entity);
        }
    }
}

// ── Python raw edge extraction ────────────────────────────────────

pub(super) fn extract_python_raw_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "import_statement" | "import_from_statement" => {
                let mut imp_cursor = node.walk();
                for child in node.named_children(&mut imp_cursor) {
                    let name = node_text(&child, source).unwrap_or_default();
                    if !name.is_empty() {
                        edges.push(RawEdge {
                            source_type: "file",
                            source_name: file_path.to_string(),
                            target_name: name,
                            edge_type: "imports",
                        });
                    }
                }
            }
            "function_definition" => {
                let func_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                if !func_name.is_empty() {
                    collect_python_calls(&node, source, "function", &func_name, edges);
                }
            }
            "class_definition" => {
                // extends: class Foo(Bar) → extends from Foo to Bar
                let class_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                if let Some(sc_node) = node.child_by_field_name("superclasses") {
                    let mut sc_cursor = sc_node.walk();
                    for sc in sc_node.named_children(&mut sc_cursor) {
                        let sc_name = node_text(&sc, source).unwrap_or_default();
                        if !sc_name.is_empty() && !class_name.is_empty() {
                            edges.push(RawEdge {
                                source_type: "class",
                                source_name: class_name.clone(),
                                target_name: sc_name,
                                edge_type: "extends",
                            });
                        }
                    }
                }
                // calls from methods inside class
                let mut class_cursor = node.walk();
                for child in node.named_children(&mut class_cursor) {
                    if child.kind() == "function_definition" {
                        let method_name = child
                            .child_by_field_name("name")
                            .and_then(|n| node_text(&n, source))
                            .unwrap_or_default();
                        if !method_name.is_empty() {
                            let qualified = format!("{class_name}::{method_name}");
                            collect_python_calls(&child, source, "function", &qualified, edges);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_python_calls(
    node: &Node,
    source: &[u8],
    source_type: &'static str,
    source_name: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut f = |desc: &Node| {
        if desc.kind() == "call"
            && let Some(func_node) = desc.child_by_field_name("function")
        {
            let call_name = node_text(&func_node, source).unwrap_or_default();
            if !call_name.is_empty() {
                edges.push(RawEdge {
                    source_type,
                    source_name: source_name.to_string(),
                    target_name: call_name,
                    edge_type: "calls",
                });
            }
        }
        true
    };
    walk_descendants(node, &mut f);
}
