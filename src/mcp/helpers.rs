//! Helper functions for MCP tool handlers.
//!
//! File-first write logic, response builders, and error helpers.
//! Separated from `tools.rs` to keep file sizes under 400 lines.

use rmcp::ErrorData as McpError;
use serde_json::json;

#[path = "entity_helpers.rs"]
mod entity_helpers;

pub use crate::mcp::update_knowledge::update_knowledge_file;
pub use entity_helpers::{create_entity_file, write_and_sync};

use crate::embed::{EmbeddingModel, OnnxEmbeddingModel};
use crate::files::EntityFile;
use crate::storage::crud::Entity;
use crate::storage::query;

// Re-export error helpers and response builders for backward-compatible imports.
pub use crate::mcp::errors::{mcp_error, mcp_internal_error, mcp_invalid_parameter};
pub use crate::mcp::responses::{context_response, query_response, search_response, tool_success};
// Re-export status builder so tools.rs imports unchanged.
pub use crate::mcp::status::build_status_response;

/// Default status filter for query tools: "active" when no status is
/// provided, matching the MCP contract.
pub const DEFAULT_QUERY_STATUS: &str = "active";

/// Maximum number of entities a single query can return. Prevents
/// unbounded result materialization from malformed or runaway MCP
/// calls.
pub const MAX_QUERY_LIMIT: i64 = 500;

/// Validate and clamp a query limit. Negative values, zero, and
/// values exceeding MAX_QUERY_LIMIT are rejected. Returns the
/// clamped limit or an MCP parameter error.
pub fn validate_query_limit(limit: i64) -> Result<i64, McpError> {
    if limit <= 0 {
        return Err(mcp_error(
            "invalid_params",
            &format!("limit must be a positive integer, got {limit}"),
        ));
    }
    if limit > MAX_QUERY_LIMIT {
        return Err(mcp_error(
            "invalid_params",
            &format!("limit {limit} exceeds maximum of {MAX_QUERY_LIMIT}"),
        ));
    }
    Ok(limit)
}

/// Type alias for the query result: entities with their references map.
pub type QueryWithRefs = (Vec<Entity>, std::collections::HashMap<String, Vec<String>>);

/// Write an entity file atomically using temp-file + rename. This
/// never truncates the canonical file in place — the original is
/// untouched until the new content is fully written and renamed into
/// place. For new files, uses `create_new` to prevent concurrent
/// writers from overwriting each other. If the path is taken by
/// another thread's write between `file_path_safe` and this call,
/// retries with `file_path_safe` to get a collision-free path.
/// Never falls back to overwriting a path owned by another entity.
pub fn write_entity_file_atomic(
    initial_path: &std::path::Path,
    entity: &EntityFile,
    cogz_dir: &std::path::Path,
) -> Result<std::path::PathBuf, std::io::Error> {
    use crate::files::read_entity_file;

    let content = entity.to_file_content();

    // If the file already exists and belongs to this entity, it's an
    // update — write to a unique temp file in the same directory, then
    // rename. The temp file uses exclusive creation (create_new) with
    // a unique name to prevent symlink-based overwrite attacks: a
    // deterministic temp name could be a symlink pointing outside .cogz.
    if initial_path.exists()
        && let Ok(existing) = read_entity_file(initial_path)
        && existing.id == entity.id
    {
        let parent = initial_path.parent().unwrap_or(std::path::Path::new("."));
        let temp_name = format!(
            ".{}.{}.tmp",
            initial_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("entity"),
            chrono::Utc::now().timestamp_millis(),
        );
        let temp_path = parent.join(&temp_name);
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        // Platform-aware atomic replace: on Unix, rename overwrites
        // atomically. On Windows, rename fails if the destination
        // exists, so remove it first.
        if cfg!(windows) {
            let _ = std::fs::remove_file(initial_path);
        }
        std::fs::rename(&temp_path, initial_path)?;
        return Ok(initial_path.to_path_buf());
        // File exists but belongs to a different entity — file_path_safe
        // should have caught this. Fall through to create_new retry.
    }

    // New file: use create_new for atomic creation. Retry up to 5
    // times if another thread creates the file between our check and
    // write. Each retry recalculates a collision-free path.
    let mut current_path = initial_path.to_path_buf();
    for _ in 0..5 {
        if let Some(parent) = current_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&current_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                if let Err(e) = file.write_all(content.as_bytes()) {
                    // Clean up partial file on write failure.
                    let _ = std::fs::remove_file(&current_path);
                    return Err(e);
                }
                return Ok(current_path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Another thread created the file. Re-run file_path_safe
                // to get a collision-free path.
                let new_path = entity.file_path_safe(cogz_dir);
                if new_path == current_path {
                    // Path didn't change — the collision is persistent.
                    // Add a unique suffix to break it.
                    let unique = format!(
                        "{}-{}",
                        entity.id.get(..8).unwrap_or(&entity.id),
                        chrono::Utc::now().timestamp_millis()
                    );
                    let slug = crate::files::entities::slugify(&entity.title);
                    let hashed = format!("{}-{}", slug, unique);
                    current_path = match entity.entity_type {
                        crate::files::FileEntityType::Knowledge => {
                            let category = entity
                                .frontmatter
                                .get("category")
                                .and_then(|v| v.as_str())
                                .unwrap_or("uncategorized");
                            let safe_category = crate::files::entities::sanitize_category(category);
                            cogz_dir
                                .join("knowledge")
                                .join(safe_category)
                                .join(format!("{}.md", hashed))
                        }
                        crate::files::FileEntityType::Rule => {
                            cogz_dir.join("rules").join(format!("{}.md", hashed))
                        }
                        crate::files::FileEntityType::Observation => current_path,
                    };
                    continue;
                }
                current_path = new_path;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "could not find a collision-free path after 5 retries: {}",
            initial_path.display()
        ),
    ))
}

/// Query entities by type with status defaulting to "active", optional
/// reference filtering, and batch-fetched references for the response.
/// Returns `(entities, references_map)`.
///
/// Status resolution: `None` → `"active"`, `"all"` → no filter, anything
/// else → literal status match. This mirrors `search::resolve_status_filter`.
pub fn query_by_type_with_refs(
    conn: &rusqlite::Connection,
    entity_type: &str,
    status: Option<&str>,
    limit: i64,
    references_filter: Option<&str>,
) -> Result<QueryWithRefs, crate::storage::StorageError> {
    let status_filter = match status {
        None => Some(DEFAULT_QUERY_STATUS),
        Some("all") => None,
        Some(s) => Some(s),
    };

    // When a references filter is provided, push it into SQL via
    // query_entities so the LIMIT applies after filtering. Without
    // this, we'd fetch `limit` entities and then discard most of
    // them, returning fewer than expected.
    //
    // For rules without a references filter, use the confidence-ordered
    // path. For rules WITH a references filter, confidence ordering is
    // sacrificed — references-filtered rule queries are a narrow case
    // and recency ordering (from query_entities) is acceptable.
    let entities = if references_filter.is_some() {
        crate::storage::query::query_entities(
            conn,
            &crate::storage::query::EntityFilter {
                entity_type: Some(entity_type),
                status: status_filter,
                file_path: None,
                references: references_filter,
                limit,
            },
        )?
    } else if entity_type == "rule" {
        crate::storage::query::get_rules_by_confidence(conn, status_filter, limit)?
    } else {
        crate::storage::query::get_entities_by_type(conn, entity_type, status_filter, limit)?
    };

    let ids: Vec<String> = entities.iter().map(|e| e.id.clone()).collect();
    let refs_map = crate::storage::graph::get_references_batch(conn, &ids)?;
    Ok((entities, refs_map))
}

/// Generate a title from content (first line, truncated).
pub fn auto_title(content: &str) -> String {
    const MAX_TITLE_CHARS: usize = 80;
    let first_line = content.lines().next().unwrap_or(content);
    if first_line.chars().count() <= MAX_TITLE_CHARS {
        return first_line.to_string();
    }
    let truncated: String = first_line.chars().take(MAX_TITLE_CHARS - 3).collect();
    format!("{truncated}...")
}

/// Embed entity text for vector storage. Returns None if the model
/// is unavailable (graceful degradation). The text is built from
/// title + content, matching the sync pipeline's `embed_entities`.
pub(crate) fn embed_entity_text(
    model: &OnnxEmbeddingModel,
    title: &str,
    content: &str,
) -> Option<Vec<f32>> {
    if !model.model_files_exist() {
        return None;
    }
    let text = if title.is_empty() {
        content.to_string()
    } else {
        format!("{}\n\n{}", title, content)
    };
    match model.embed(&[&text]) {
        Ok(embeddings) => embeddings.into_iter().next(),
        Err(e) => {
            tracing::warn!("entity embedding failed: {}", e);
            None
        }
    }
}

/// Create an entity file, write it, sync to DB, embed, and run dedup
/// and contradiction checks. The `EntityFile` is built by the caller
/// with all frontmatter set.
/// Embed a query string for hybrid search. Returns None if the
/// embedding model is unavailable (graceful degradation to FTS-only).
/// Uses the provided persistent model to avoid reloading from disk
/// on every call.
pub fn embed_query_for_search(
    model: &crate::embed::OnnxEmbeddingModel,
    query: &str,
) -> Option<Vec<f32>> {
    if !model.model_files_exist() {
        return None;
    }
    match model.embed_query(&[query]) {
        Ok(embeddings) => embeddings.into_iter().next(),
        Err(e) => {
            tracing::warn!("query embedding failed: {}", e);
            None
        }
    }
}

/// Build the `list_entities` response JSON. Queries entities by type
/// with optional status filter, returns IDs and titles only.
pub fn list_entities_response(
    conn: &rusqlite::Connection,
    entity_type: &str,
    status: Option<&str>,
) -> Result<serde_json::Value, crate::storage::StorageError> {
    let status_filter = match status {
        None => Some(DEFAULT_QUERY_STATUS),
        Some("all") => None,
        Some(s) => Some(s),
    };
    let entities = query::get_entities_by_type(conn, entity_type, status_filter, 1000)?;
    let items: Vec<_> = entities
        .iter()
        .map(|e| json!({ "id": e.id, "title": e.title }))
        .collect();
    Ok(json!({
        "entity_type": entity_type,
        "entities": items,
        "count": items.len(),
    }))
}

/// Query knowledge entries with optional filters. Returns
/// `(entities, references_map)` for the response builder.
pub fn query_knowledge_with_refs(
    conn: &rusqlite::Connection,
    status: Option<&str>,
    category: Option<&str>,
    tags: Option<&[String]>,
    limit: i64,
) -> Result<QueryWithRefs, crate::storage::StorageError> {
    let status_filter = match status {
        None => Some(DEFAULT_QUERY_STATUS),
        Some("all") => None,
        Some(s) => Some(s),
    };
    let entities = query::get_knowledge_filtered(conn, status_filter, category, tags, limit)?;
    let ids: Vec<String> = entities.iter().map(|e| e.id.clone()).collect();
    let refs_map = crate::storage::graph::get_references_batch(conn, &ids)?;
    Ok((entities, refs_map))
}

/// Parse and validate a context mode string. Returns the parsed mode
/// or an MCP error if the mode is invalid or the query is missing.
pub fn parse_context_mode(
    mode_str: &str,
    query: Option<&str>,
) -> Result<crate::context::ContextMode, McpError> {
    let mode = crate::context::ContextMode::parse(mode_str).ok_or_else(|| {
        mcp_invalid_parameter(&format!(
            "invalid mode '{}': expected cold_start, task, or escalation",
            mode_str
        ))
    })?;
    if mode.requires_query() {
        let q = query.unwrap_or("").trim();
        if q.is_empty() {
            return Err(mcp_invalid_parameter(&format!(
                "query is required for {} mode",
                mode
            )));
        }
    }
    Ok(mode)
}
