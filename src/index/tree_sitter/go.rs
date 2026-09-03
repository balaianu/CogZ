//! Go AST extraction — functions, methods, types, imports.

use serde_json::json;
use tree_sitter::Node;

use super::{CodeEntity, RawEdge, node_text, walk_descendants};

pub(super) fn extract_go(
    root: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    // Package clause → module entity
    let mut root_cursor = root.walk();
    for child in root.named_children(&mut root_cursor) {
        if child.kind() == "package_clause" {
            let mut pkg_cursor = child.walk();
            for pkg_child in child.named_children(&mut pkg_cursor) {
                if pkg_child.kind() == "package_identifier"
                    && let Some(name) = node_text(&pkg_child, source)
                {
                    entities.push(CodeEntity {
                        entity_type: "module",
                        title: name.clone(),
                        content: node_text(&child, source).unwrap_or_default(),
                        properties: json!({
                            "file_path": file_path,
                            "language": "go",
                            "module_path": name,
                        }),
                    });
                }
            }
        }
    }

    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(entity) = extract_go_function(&child, source, file_path, None) {
                    entities.push(entity);
                }
            }
            "method_declaration" => {
                if let Some(entity) = extract_go_function(&child, source, file_path, Some(&child)) {
                    entities.push(entity);
                }
            }
            "type_declaration" => {
                extract_go_types(&child, source, file_path, entities);
            }
            _ => {}
        }
    }
}

fn extract_go_function(
    node: &Node,
    source: &[u8],
    file_path: &str,
    method_node: Option<&Node>,
) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;
    let signature = content.lines().next().unwrap_or("").to_string();

    let (qualified_name, receiver_type) = match method_node {
        Some(m) => {
            // receiver is parameter_list → parameter_declaration → type field
            let recv = m
                .child_by_field_name("receiver")
                .and_then(|r| {
                    let param = r.named_child(0)?;
                    param
                        .child_by_field_name("type")
                        .and_then(|t| node_text(&t, source))
                })
                .unwrap_or_default();
            (format!("{recv}::{name}"), Some(recv))
        }
        None => (name.clone(), None),
    };

    let title = match &receiver_type {
        Some(rt) => format!("{}::{}", rt, name),
        None => name.clone(),
    };

    Some(CodeEntity {
        entity_type: "function",
        title,
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "go",
            "signature": signature,
            "qualified_name": qualified_name,
            "receiver_type": receiver_type,
        }),
    })
}

fn extract_go_types(
    type_decl: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let mut cursor = type_decl.walk();
    for child in type_decl.named_children(&mut cursor) {
        if child.kind() == "type_spec"
            && let Some(entity) = extract_go_type(&child, source, file_path)
        {
            entities.push(entity);
        }
    }
}

fn extract_go_type(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    let kind = node
        .child_by_field_name("type")
        .map(|t| t.kind())
        .unwrap_or("type");

    let kind_str = match kind {
        "struct_type" => "struct",
        "interface_type" => "interface",
        _ => "type_alias",
    };

    Some(CodeEntity {
        entity_type: "class",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "go",
            "qualified_name": name,
            "kind": kind_str,
        }),
    })
}

// ── Go raw edge extraction ─────────────────────────────────────────

pub(super) fn extract_go_raw_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "import_declaration" => {
                // import_declaration contains import_spec children
                let mut imp_cursor = node.walk();
                for child in node.named_children(&mut imp_cursor) {
                    if child.kind() == "import_spec" {
                        // The path is in a "path" field (string literal)
                        if let Some(path_node) = child.child_by_field_name("path") {
                            let path = node_text(&path_node, source)
                                .unwrap_or_default()
                                .trim_matches('"')
                                .to_string();
                            if !path.is_empty() {
                                // Use the last segment as the import name
                                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                                edges.push(RawEdge {
                                    source_type: "file",
                                    source_name: file_path.to_string(),
                                    target_name: name,
                                    edge_type: "imports",
                                });
                            }
                        }
                    }
                }
            }
            "function_declaration" => {
                let func_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                if !func_name.is_empty() {
                    collect_go_calls(&node, source, "function", &func_name, edges);
                }
            }
            "method_declaration" => {
                let func_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                let receiver = node
                    .child_by_field_name("receiver")
                    .and_then(|r| {
                        let param = r.named_child(0)?;
                        param
                            .child_by_field_name("type")
                            .and_then(|t| node_text(&t, source))
                    })
                    .unwrap_or_default();
                if !func_name.is_empty() && !receiver.is_empty() {
                    let qualified = format!("{receiver}::{func_name}");
                    collect_go_calls(&node, source, "function", &qualified, edges);
                }
            }
            "type_declaration" => {
                let mut td_cursor = node.walk();
                for spec in node.named_children(&mut td_cursor) {
                    if spec.kind() == "type_spec" {
                        let type_name = spec
                            .child_by_field_name("name")
                            .and_then(|n| node_text(&n, source))
                            .unwrap_or_default();
                        if !type_name.is_empty() {
                            // Check for interface embedding or struct embedding
                            if let Some(type_node) = spec.child_by_field_name("type") {
                                collect_go_embeds(&type_node, source, &type_name, edges);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_go_calls(
    node: &Node,
    source: &[u8],
    source_type: &'static str,
    source_name: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut f = |desc: &Node| {
        if desc.kind() == "call_expression"
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

fn collect_go_embeds(type_node: &Node, source: &[u8], source_name: &str, edges: &mut Vec<RawEdge>) {
    // Struct embedding: struct { io.Reader } → extends io.Reader
    // Interface embedding: interface { io.Reader } → extends io.Reader
    let mut f = |desc: &Node| {
        if desc.kind() == "qualified_type" || desc.kind() == "type_identifier" {
            let name = node_text(desc, source).unwrap_or_default();
            if !name.is_empty() {
                edges.push(RawEdge {
                    source_type: "class",
                    source_name: source_name.to_string(),
                    target_name: name,
                    edge_type: "extends",
                });
            }
        }
        true
    };
    walk_descendants(type_node, &mut f);
}
