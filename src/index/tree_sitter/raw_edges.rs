//! Rust raw edge extraction — calls, imports, extends.
//!
//! Produces `RawEdge` values with unresolved name references from a
//! single AST walk. The `code_graph` module resolves these to UUIDs.

use tree_sitter::Node;

use super::{RawEdge, node_text, walk_descendants};

pub(super) fn extract_rust_raw_edges(
    root: &Node,
    source: &[u8],
    file_path: &str,
    edges: &mut Vec<RawEdge>,
) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "use_declaration" => {
                if let Some(arg) = node.child_by_field_name("argument") {
                    let import_text = node_text(&arg, source).unwrap_or_default();
                    if !import_text.is_empty() {
                        edges.push(RawEdge {
                            source_type: "file",
                            source_name: file_path.to_string(),
                            target_name: import_text,
                            edge_type: "imports",
                        });
                    }
                }
            }
            "function_item" => {
                let func_name = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                if !func_name.is_empty() {
                    collect_rust_calls(&node, source, "function", &func_name, edges);
                }
            }
            "impl_item" => {
                // extends: impl Trait for Type
                let type_name = node
                    .child_by_field_name("type")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or_default();
                let trait_name = node
                    .child_by_field_name("trait")
                    .and_then(|n| node_text(&n, source));
                if let Some(tr) = trait_name.as_ref() {
                    edges.push(RawEdge {
                        source_type: "class",
                        source_name: type_name.clone(),
                        target_name: tr.clone(),
                        edge_type: "extends",
                    });
                }
                // calls from methods inside impl
                let mut impl_cursor = node.walk();
                for child in node.named_children(&mut impl_cursor) {
                    if child.kind() == "function_item" {
                        let method_name = child
                            .child_by_field_name("name")
                            .and_then(|n| node_text(&n, source))
                            .unwrap_or_default();
                        if !method_name.is_empty() {
                            let qualified = format!("{type_name}::{method_name}");
                            collect_rust_calls(&child, source, "function", &qualified, edges);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_rust_calls(
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
