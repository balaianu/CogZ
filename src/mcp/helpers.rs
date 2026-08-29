//! Helper functions for MCP tool handlers.
//!
//! File-first write logic, response builders, and error helpers.
//! Separated from `tools.rs` to keep file sizes under 400 lines.

use std::sync::Arc;

use rmcp::ErrorData as McpError;
use serde_json::json;

use crate::config::Config;
use crate::embed::{EmbeddingModel, OnnxEmbeddingModel};
use crate::files::embed_sync::store_embeddings;
use crate::files::frontmatter::FmValue;
use crate::files::{EntityFile, read_entity_file, sync_incremental, write_entity_file};
use crate::mcp::dedup::check_duplicate;
use crate::mcp::params::UpdateKnowledgeParams;
use crate::storage::{Storage, crud, events};

// Re-export error helpers and response builders for backward-compatible imports.
pub use crate::mcp::errors::{mcp_error, mcp_internal_error, mcp_invalid_parameter};
pub use crate::mcp::responses::{context_response, query_response, search_response, tool_success};
// Re-export status builder so tools.rs imports unchanged.
pub use crate::mcp::status::build_status_response;

/// Default status filter for query tools: "active" when no status is
/// provided, matching the MCP contract.
pub const DEFAULT_QUERY_STATUS: &str = "active";

/// Type alias for the query result: entities with their references map.
pub type QueryWithRefs = (
    Vec<crud::Entity>,
    std::collections::HashMap<String, Vec<String>>,
);

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
fn embed_entity_text(model: &OnnxEmbeddingModel, title: &str, content: &str) -> Option<Vec<f32>> {
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

/// Create an entity file, write it, sync to DB, embed, and run dedup.
/// The `EntityFile` is built by the caller with all frontmatter set.
pub fn create_entity_file(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &std::path::Path,
    entity: &EntityFile,
    embed_model: Option<&OnnxEmbeddingModel>,
) -> Result<serde_json::Value, McpError> {
    write_and_sync(
        storage,
        config,
        cogz_dir,
        entity,
        entity.entity_type.as_str(),
        embed_model,
    )
}

/// Write an entity file and sync it to the DB. Returns the tool
/// response JSON with id, file_path, status, and dedup results.
pub fn write_and_sync(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &std::path::Path,
    entity: &EntityFile,
    entity_type: &str,
    embed_model: Option<&OnnxEmbeddingModel>,
) -> Result<serde_json::Value, McpError> {
    // 1. Write file first (file-first invariant)
    let path = entity.file_path(cogz_dir);
    write_entity_file(&path, entity).map_err(|e| {
        mcp_error(
            "file_write_failed",
            &format!("Failed to write entity file: {}", e),
        )
    })?;

    // 2. Sync to DB (incremental — only the new/changed file)
    let sync_result = sync_incremental(storage, cogz_dir);
    if !sync_result.errors.is_empty() {
        let err = &sync_result.errors[0];
        return Err(mcp_error(
            "db_error",
            &format!("Sync failed: {}", err.error),
        ));
    }

    // 3. Embed (no DB lock held during ONNX inference)
    let embedding = embed_model.and_then(|m| embed_entity_text(m, &entity.title, &entity.body));

    // 4. Store embedding + run dedup (under lock, no I/O)
    let dedup = {
        let mut conn = storage.conn();
        if let Some(ref emb) = embedding {
            store_embeddings(&mut conn, &[(entity.id.clone(), emb.clone())]);
        }
        check_duplicate(
            &conn,
            &entity.id,
            &entity.title,
            entity_type,
            embedding.as_deref(),
            &config.consolidation,
        )
    };

    // Sync already records the creation event via files::events::record_create_event.
    // We only need to record an additional event if dedup flagged something.
    if dedup.dedup_flagged {
        let conn = storage.conn();
        let event_type = match entity.entity_type {
            crate::files::FileEntityType::Observation => events::EventType::ObservationCreated,
            crate::files::FileEntityType::Rule => events::EventType::RuleCreated,
            crate::files::FileEntityType::Knowledge => events::EventType::KnowledgeCreated,
        };
        let payload = json!({"dedup_flagged": true});
        let _ = events::record_event(&conn, event_type, Some(&entity.id), &payload);
    }

    let relative_path = path
        .strip_prefix(cogz_dir.parent().unwrap_or(cogz_dir))
        .unwrap_or(&path);

    Ok(json!({
        "id": entity.id,
        "file_path": relative_path.display().to_string(),
        "status": entity.status,
        "dedup_flagged": dedup.dedup_flagged,
        "contradiction_flagged": dedup.contradiction_flagged,
        "duplicate_warning": dedup.duplicate_warning,
    }))
}

/// Update a knowledge file in-place: read existing, modify, write, sync.
pub fn update_knowledge_file(
    storage: &Arc<Storage>,
    cogz_dir: &std::path::Path,
    params: &UpdateKnowledgeParams,
    embed_model: Option<&OnnxEmbeddingModel>,
) -> Result<serde_json::Value, McpError> {
    // 1. Get the existing entity from DB to find its file path
    let entity = {
        let conn = storage.conn();
        crud::get_entity(&conn, &params.id).map_err(|e| match e {
            crate::storage::StorageError::EntityNotFound(_) => mcp_error(
                "entity_not_found",
                &format!("Entity {} not found", params.id),
            ),
            other => mcp_error("db_error", &other.to_string()),
        })?
    };

    if entity.r#type != "knowledge" {
        return Err(mcp_error(
            "invalid_parameter",
            &format!("Entity {} is not a knowledge entry", params.id),
        ));
    }

    // 2. Read the existing file
    let file_path = entity
        .file_path
        .as_ref()
        .ok_or_else(|| mcp_error("entity_not_found", "Entity has no file path"))?;
    let abs_path = if file_path.starts_with(".cogz") {
        cogz_dir.parent().unwrap_or(cogz_dir).join(file_path)
    } else {
        cogz_dir.join(file_path)
    };

    let mut entity_file = read_entity_file(&abs_path)
        .map_err(|e| mcp_error("file_write_failed", &format!("Failed to read file: {}", e)))?;

    // 3. Modify fields
    let mut updated_fields = Vec::new();
    entity_file.body = params.content.clone();
    updated_fields.push("content");
    if let Some(ref title) = params.title {
        entity_file.title = title.clone();
        updated_fields.push("title");
    }
    if let Some(ref category) = params.category {
        entity_file
            .frontmatter
            .insert("category", FmValue::String(category.clone()));
        updated_fields.push("category");
    }
    if let Some(ref tags) = params.tags {
        entity_file
            .frontmatter
            .insert("tags", FmValue::Array(tags.clone()));
        updated_fields.push("tags");
    }
    if let Some(ref refs) = params.references {
        entity_file.references = refs.clone();
        updated_fields.push("references");
    }
    entity_file.updated_at = chrono::Utc::now().to_rfc3339();

    // 4. Write the file (may move to new path if category changed)
    let new_path = entity_file.file_path(cogz_dir);
    if new_path != abs_path
        && let Err(e) = std::fs::remove_file(&abs_path)
    {
        tracing::warn!(
            "failed to remove old entity file {}: {}",
            abs_path.display(),
            e
        );
    }
    write_entity_file(&new_path, &entity_file)
        .map_err(|e| mcp_error("file_write_failed", &format!("Failed to write file: {}", e)))?;

    // 5. Sync to DB
    let sync_result = sync_incremental(storage, cogz_dir);
    if !sync_result.errors.is_empty() {
        let err = &sync_result.errors[0];
        return Err(mcp_error(
            "db_error",
            &format!("Sync failed: {}", err.error),
        ));
    }

    // 6. Re-embed (no DB lock during ONNX inference)
    let embedding =
        embed_model.and_then(|m| embed_entity_text(m, &entity_file.title, &entity_file.body));

    // 7. Store embedding + record update event (under lock)
    {
        let mut conn = storage.conn();
        if let Some(ref emb) = embedding {
            store_embeddings(&mut conn, &[(entity_file.id.clone(), emb.clone())]);
        }
        let payload = json!({"updated_fields": updated_fields});
        let _ = events::record_event(
            &conn,
            events::EventType::KnowledgeUpdated,
            Some(&entity_file.id),
            &payload,
        );
    }

    let relative_path = new_path
        .strip_prefix(cogz_dir.parent().unwrap_or(cogz_dir))
        .unwrap_or(&new_path);

    Ok(json!({
        "id": entity_file.id,
        "file_path": relative_path.display().to_string(),
        "status": entity_file.status,
        "updated_fields": updated_fields,
    }))
}

// ─── Response builders ─────────────────────────────────────────────
// Response builder functions live in `responses.rs` and are re-exported above.

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
    match model.embed(&[query]) {
        Ok(embeddings) => embeddings.into_iter().next(),
        Err(e) => {
            tracing::warn!("query embedding failed: {}", e);
            None
        }
    }
}
