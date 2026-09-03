//! Rust AST extraction — functions, types, impls, modules.

use serde_json::json;
use tree_sitter::Node;

use super::{CodeEntity, node_text};

pub(super) fn extract_rust(
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
