//! Python structural edge extraction — imports, calls, extends.

use std::collections::HashMap;

use tree_sitter::Node;

use super::{CodeEdge, code_entity_uuid, node_text, walk_descendants};

pub(super) fn extract_python_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    let file_uuid = code_entity_uuid(file_path, "file", file_path);

    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "import_statement" | "import_from_statement" => {
                extract_python_import(&node, source, &file_uuid, name_to_uuid, edges);
            }
            "function_definition" => {
                extract_python_calls_in_node(&node, source, file_path, name_to_uuid, edges);
            }
            "class_definition" => {
                extract_python_extends_in_class(&node, source, name_to_uuid, edges);
                let mut class_cursor = node.walk();
                for child in node.named_children(&mut class_cursor) {
                    if child.kind() == "function_definition" {
                        extract_python_calls_in_node(
                            &child,
                            source,
                            file_path,
                            name_to_uuid,
                            edges,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_python_import(
    node: &Node,
    source: &[u8],
    file_uuid: &str,
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name = node_text(&child, source).unwrap_or_default();
        if let Some(target_id) = name_to_uuid.get(&name) {
            edges.push(CodeEdge {
                source_id: file_uuid.to_string(),
                target_id: target_id.clone(),
                edge_type: "imports",
            });
        } else if let Some(last) = name.rsplit('.').next()
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

fn extract_python_calls_in_node(
    node: &Node,
    source: &[u8],
    file_path: &str,
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    let func_name = node
        .child_by_field_name("name")
        .and_then(|n| node_text(&n, source))
        .unwrap_or_default();

    let source_id = name_to_uuid.get(&func_name).cloned().or_else(|| {
        let qualified = format!("{file_path}::{func_name}");
        name_to_uuid.get(&qualified).cloned()
    });

    let source_id = match source_id {
        Some(id) => id,
        None => return,
    };

    let mut f = |desc: &Node| {
        if desc.kind() == "call"
            && let Some(func_node) = desc.child_by_field_name("function")
        {
            let call_name = node_text(&func_node, source).unwrap_or_default();
            if let Some(target_id) = name_to_uuid.get(&call_name) {
                edges.push(CodeEdge {
                    source_id: source_id.clone(),
                    target_id: target_id.clone(),
                    edge_type: "calls",
                });
            } else if let Some(last) = call_name.rsplit('.').next()
                && let Some(target_id) = name_to_uuid.get(last)
            {
                edges.push(CodeEdge {
                    source_id: source_id.clone(),
                    target_id: target_id.clone(),
                    edge_type: "calls",
                });
            }
        }
        true
    };
    walk_descendants(node, &mut f);
}

fn extract_python_extends_in_class(
    node: &Node,
    source: &[u8],
    name_to_uuid: &HashMap<String, String>,
    edges: &mut Vec<CodeEdge>,
) {
    let name_node = node.child_by_field_name("name");
    let superclasses_node = node.child_by_field_name("superclasses");

    let (name_n, sc_n) = match (name_node, superclasses_node) {
        (Some(n), Some(s)) => (n, s),
        _ => return,
    };

    let class_name = node_text(&name_n, source).unwrap_or_default();
    let source_id = match name_to_uuid.get(&class_name) {
        Some(id) => id.clone(),
        None => return,
    };

    let mut cursor = sc_n.walk();
    for sc in sc_n.named_children(&mut cursor) {
        let sc_name = node_text(&sc, source).unwrap_or_default();
        if let Some(target_id) = name_to_uuid.get(&sc_name) {
            edges.push(CodeEdge {
                source_id: source_id.clone(),
                target_id: target_id.clone(),
                edge_type: "extends",
            });
        }
    }
}
