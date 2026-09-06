use serde_json::json;
use tree_sitter::Node;

use super::{CodeEntity, node_text};

pub(super) fn extract_ts_interface(
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
            "kind": "interface",
        }),
    })
}

pub(super) fn extract_ts_enum(
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
            "kind": "enum",
        }),
    })
}
