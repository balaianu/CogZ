//! Bash AST extraction — functions, source commands, calls.

use serde_json::json;
use tree_sitter::Node;

use super::{CodeEntity, RawEdge, node_text, walk_descendants};

pub(super) fn extract_bash(
    root: &Node,
    source: &[u8],
    file_path: &str,
    entities: &mut Vec<CodeEntity>,
) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "function_definition"
            && let Some(entity) = extract_bash_function(&child, source, file_path)
        {
            entities.push(entity);
        }
    }
}

fn extract_bash_function(node: &Node, source: &[u8], file_path: &str) -> Option<CodeEntity> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source)?;
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;
    let content = node_text(node, source)?;

    Some(CodeEntity {
        entity_type: "function",
        title: name.clone(),
        content,
        properties: json!({
            "file_path": file_path,
            "line_start": line_start,
            "line_end": line_end,
            "language": "bash",
            "qualified_name": name,
        }),
    })
}

// ── Bash raw edge extraction ───────────────────────────────────────

pub(super) fn extract_bash_raw_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() == "function_definition" {
            let func_name = node
                .child_by_field_name("name")
                .and_then(|n| node_text(&n, source))
                .unwrap_or_default();
            if !func_name.is_empty() {
                collect_bash_calls(&node, source, "function", &func_name, edges);
            }
        } else if node.kind() == "command" {
            // Top-level commands: source ./file.sh → import
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = node_text(&name_node, source)
                && (name == "source" || name == ".")
            {
                let mut cmd_cursor = node.walk();
                for arg in node.named_children(&mut cmd_cursor) {
                    if arg.kind() == "word" || arg.kind() == "string" {
                        let arg_text = node_text(&arg, source).unwrap_or_default();
                        let cleaned = arg_text.trim_matches(|c: char| c == '"' || c == '\'');
                        if !cleaned.is_empty() {
                            let import_name = cleaned
                                .rsplit('/')
                                .next()
                                .unwrap_or(cleaned)
                                .split('.')
                                .next()
                                .unwrap_or(cleaned)
                                .to_string();
                            edges.push(RawEdge {
                                source_type: "file",
                                source_name: file_path.to_string(),
                                target_name: import_name,
                                edge_type: "imports",
                            });
                        }
                        break;
                    }
                }
            }
            collect_bash_command_calls(&node, source, "file", file_path, edges);
        }
    }
}

fn collect_bash_calls(
    node: &Node,
    source: &[u8],
    source_type: &'static str,
    source_name: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut f = |desc: &Node| {
        if desc.kind() == "command"
            && let Some(name_node) = desc.child_by_field_name("name")
            && let Some(call_name) = node_text(&name_node, source)
            && !is_bash_keyword(&call_name)
            && !call_name.is_empty()
        {
            edges.push(RawEdge {
                source_type,
                source_name: source_name.to_string(),
                target_name: call_name,
                edge_type: "calls",
            });
        }
        true
    };
    walk_descendants(node, &mut f);
}

fn collect_bash_command_calls(
    node: &Node,
    source: &[u8],
    source_type: &'static str,
    source_name: &str,
    edges: &mut Vec<RawEdge>,
) {
    if let Some(name_node) = node.child_by_field_name("name")
        && let Some(call_name) = node_text(&name_node, source)
        && !is_bash_keyword(&call_name)
        && !call_name.is_empty()
    {
        edges.push(RawEdge {
            source_type,
            source_name: source_name.to_string(),
            target_name: call_name,
            edge_type: "calls",
        });
    }
}

fn is_bash_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "do"
            | "done"
            | "while"
            | "until"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "return"
            | "break"
            | "continue"
            | "local"
            | "export"
            | "readonly"
            | "declare"
            | "unset"
            | "true"
            | "false"
            | "exit"
            | "shift"
            | "source"
            | "."
            | "echo"
            | "printf"
            | "read"
            | "set"
            | "trap"
            | "eval"
            | "exec"
            | "test"
            | "["
            | "[["
    )
}
