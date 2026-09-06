use tree_sitter::Node;

use super::{RawEdge, node_text, walk_descendants};

pub(super) fn collect_js_imports(
    node: &Node,
    source: &[u8],
    file_path: &str,
    edges: &mut Vec<RawEdge>,
) {
    // import_statement children: import_clause (named), string (source)
    // The string is the module specifier. Resolve it against the
    // importing file's directory so the edge target matches the file
    // entity's file_path key (e.g. "src/utils.js", not "utils").
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "string" {
            let spec = node_text(&child, source)
                .unwrap_or_default()
                .trim_matches(|c: char| c == '"' || c == '\'')
                .to_string();
            if spec.is_empty() {
                continue;
            }

            // Resolve relative specifiers (./foo, ../foo) against the
            // importing file's directory. Bare specifiers (react, lodash)
            // are external packages — skip them, they have no file entity.
            let resolved = if spec.starts_with("./") || spec.starts_with("../") {
                resolve_relative_module(file_path, &spec)
            } else {
                // Bare specifier — external package, no file entity to
                // link to. Skip rather than creating a dangling edge.
                continue;
            };

            if let Some(target_path) = resolved {
                edges.push(RawEdge {
                    source_type: "file",
                    source_name: file_path.to_string(),
                    target_name: target_path,
                    edge_type: "imports",
                });
            }
        }
    }
}

/// Resolve a relative module specifier (./foo, ../foo) against the
/// importing file's directory. Returns the normalized file path
/// with a supported extension.
///
/// Since the extractor has no filesystem access, it cannot check which
/// extension the target file actually uses. It emits the `.ts` variant
/// for extensionless specifiers (most common in modern TS projects).
/// If the actual file is `.js`, the edge won't resolve — but this is
/// strictly better than the old behavior of reducing to a bare name
/// that could match an unrelated function/class.
///
/// Handles:
/// - `./utils` → `src/utils.ts`
/// - `./utils.js` → `src/utils.js`
/// - `../lib/helper` → `lib/helper.ts`
/// - `./components/` → `src/components/index.ts`
fn resolve_relative_module(importer_path: &str, spec: &str) -> Option<String> {
    let dir = importer_path
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or(".");
    let joined = join_paths(dir, spec);

    // If the specifier already has a supported extension, use it as-is.
    if has_js_extension(&joined) {
        return Some(normalize_path(&joined));
    }

    // Directory import (ends with /): try index.ts
    if joined.ends_with('/') {
        let index_path = format!("{}index.ts", joined);
        return Some(normalize_path(&index_path));
    }

    // Extensionless specifier: default to .ts
    Some(normalize_path(&format!("{}.ts", joined)))
}

fn join_paths(dir: &str, spec: &str) -> String {
    if let Some(rest) = spec.strip_prefix("./") {
        format!("{}/{}", dir, rest)
    } else if spec.starts_with("../") {
        // Handle parent directory traversal.
        let mut parts: Vec<&str> = dir.split('/').collect();
        let spec_parts: Vec<&str> = spec.split('/').collect();
        for sp in &spec_parts {
            if *sp == ".." {
                parts.pop();
            } else if *sp != "." && !sp.is_empty() {
                parts.push(sp);
            }
        }
        parts.join("/")
    } else {
        spec.to_string()
    }
}

fn normalize_path(path: &str) -> String {
    // Collapse consecutive slashes and ./ components.
    let mut result = String::new();
    for component in path.split('/') {
        if component == "." || component.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push('/');
        }
        result.push_str(component);
    }
    result
}

fn has_js_extension(path: &str) -> bool {
    [".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

pub(super) fn collect_js_calls(
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

/// Collect call edges from variable-declared arrow functions and
/// function expressions. Iterates `variable_declarator` children,
/// finds those whose value is an `arrow_function` or
/// `function_expression`, and collects calls from the function body.
pub(super) fn collect_js_variable_calls(node: &Node, source: &[u8], edges: &mut Vec<RawEdge>) {
    let mut cursor = node.walk();
    for decl in node.named_children(&mut cursor) {
        if decl.kind() != "variable_declarator" {
            continue;
        }
        let name = match decl
            .child_by_field_name("name")
            .and_then(|n| node_text(&n, source))
        {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        let value_node = match decl.child_by_field_name("value") {
            Some(v) => v,
            None => continue,
        };
        if !matches!(value_node.kind(), "arrow_function" | "function_expression") {
            continue;
        }
        collect_js_calls(&value_node, source, "function", &name, edges);
    }
}
