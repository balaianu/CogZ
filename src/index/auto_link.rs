//! Auto-linking — automatically connect knowledge entities to code
//! entities by scanning content for file paths and symbol names.
//!
//! Two strategies:
//! 1. Path-based: scan content for file paths (e.g. `src/embed/onnx.rs`)
//!    and link to the corresponding file entity.
//! 2. Name-based: scan content for code entity titles (function/class
//!    names) and link to matching entities. Only unique names are
//!    linked to avoid false positives from common names.
//!
//! All auto-links are `references` edges from the knowledge entity to
//! the code entity. They are DB-only (not in frontmatter) and fully
//! rebuildable on reindex.

use std::collections::HashMap;

use crate::storage;
use crate::storage::edges::Edge;
use crate::storage::query::get_entities_by_type;

/// Sync auto-links: scan all knowledge entities (observation, rule,
/// knowledge) for references to code entities and create `references`
/// edges. Existing auto-links are cleared and rebuilt.
///
/// Must be called after both file sync and code sync are complete so
/// that all entities exist in the DB.
pub fn sync_auto_links(storage: &storage::Storage) -> usize {
    let conn = storage.conn();

    // Clear existing auto-linked references edges. We distinguish
    // auto-links from manual references by tracking them in a separate
    // edge_type: "auto_references". This preserves manual `references`
    // edges from frontmatter while allowing auto-links to be rebuilt.
    if let Err(e) = storage::edges::delete_edges_by_type(&conn, &["auto_references"]) {
        tracing::warn!("failed to clear auto-link edges: {}", e);
    }

    // Build name → code entity ID map. Only include names that are
    // unique across the codebase to avoid false positives.
    let code_types = ["function", "class", "file", "module"];
    let mut name_count: HashMap<String, usize> = HashMap::new();
    let mut name_to_id: HashMap<String, String> = HashMap::new();

    for entity_type in &code_types {
        if let Ok(entities) = get_entities_by_type(&conn, entity_type, None, 100_000) {
            for entity in entities {
                if let Some(ref title) = entity.title {
                    *name_count.entry(title.clone()).or_default() += 1;
                    name_to_id.entry(title.clone()).or_insert(entity.id.clone());
                }
            }
        }
    }

    // Filter to unique names only. Also build a set of file paths
    // for path-based matching.
    let unique_names: HashMap<String, String> = name_to_id
        .into_iter()
        .filter(|(name, _)| {
            let count = name_count.get(name).copied().unwrap_or(0);
            // Link by full title if unique. For file entities, the
            // title is the file name (e.g. "onnx.rs") which may not
            // be unique — require the full path instead.
            count == 1
        })
        .collect();

    // Build file path → file entity ID map for path-based matching.
    let mut path_to_id: HashMap<String, String> = HashMap::new();
    if let Ok(files) = get_entities_by_type(&conn, "file", None, 100_000) {
        for file in files {
            if let Some(ref fp) = file.file_path {
                path_to_id.insert(fp.clone(), file.id.clone());
            }
        }
    }

    // Scan knowledge entities for references.
    let knowledge_types = ["observation", "rule", "knowledge"];
    let mut edges: Vec<Edge> = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();

    for kt in &knowledge_types {
        if let Ok(entities) = get_entities_by_type(&conn, kt, Some("active"), 100_000) {
            for entity in entities {
                let content = format!(
                    "{}\n{}",
                    entity.title.as_deref().unwrap_or(""),
                    entity.content
                );

                // Path-based: find file paths in content.
                for (path, id) in &path_to_id {
                    if content.contains(path.as_str()) {
                        edges.push(Edge {
                            source_id: entity.id.clone(),
                            target_id: id.clone(),
                            edge_type: "auto_references".to_string(),
                            weight: 1.0,
                            created_at: now.clone(),
                        });
                    }
                }

                // Name-based: find unique code entity names in content.
                for (name, id) in &unique_names {
                    // Skip very short names (< 4 chars) to reduce
                    // false positives from common words matching
                    // function names like "run" or "get".
                    if name.len() < 4 {
                        continue;
                    }
                    if content.contains(name.as_str()) {
                        edges.push(Edge {
                            source_id: entity.id.clone(),
                            target_id: id.clone(),
                            edge_type: "auto_references".to_string(),
                            weight: 1.0,
                            created_at: now.clone(),
                        });
                    }
                }
            }
        }
    }

    // Deduplicate edges (a knowledge entity may match both path and name).
    edges.sort_by(|a, b| (&a.source_id, &a.target_id).cmp(&(&b.source_id, &b.target_id)));
    edges.dedup_by(|a, b| a.source_id == b.source_id && a.target_id == b.target_id);

    let count = edges.len();

    if let Err(e) = conn.execute_batch("BEGIN") {
        tracing::warn!(
            "failed to begin auto-link transaction: {} — falling back to autocommit",
            e
        );
    }
    for edge in &edges {
        if let Err(e) = storage::edges::insert_edge_skip_fk_violation(&conn, edge) {
            tracing::debug!("skipped auto-link edge: {}", e);
        }
    }
    if let Err(e) = conn.execute_batch("COMMIT") {
        tracing::warn!("failed to commit auto-link transaction: {}", e);
    }

    count
}
