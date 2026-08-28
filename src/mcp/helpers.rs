//! Helper functions for MCP tool handlers.
//!
//! File-first write logic, response builders, and error helpers.
//! Separated from `tools.rs` to keep file sizes under 400 lines.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, ContentBlock, ErrorCode},
};
use serde_json::json;

use crate::config::Config;
use crate::files::frontmatter::FmValue;
use crate::files::{EntityFile, read_entity_file, sync_incremental, write_entity_file};
use crate::mcp::dedup::check_duplicate;
use crate::mcp::params::UpdateKnowledgeParams;
use crate::search::SearchResults;
use crate::storage::{Storage, crud, events};

/// Generate a title from content (first line, truncated).
pub fn auto_title(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or(content);
    if first_line.len() > 80 {
        format!("{}...", &first_line[..77])
    } else {
        first_line.to_string()
    }
}

/// Create an entity file, write it, sync to DB, and run dedup checks.
/// The `EntityFile` is built by the caller with all frontmatter set.
pub fn create_entity_file(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &std::path::Path,
    entity: &EntityFile,
) -> Result<serde_json::Value, McpError> {
    write_and_sync(
        storage,
        config,
        cogz_dir,
        entity,
        entity.entity_type.as_str(),
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

    // 3. Run dedup checks
    let dedup = {
        let conn = storage.conn();
        check_duplicate(
            &conn,
            &entity.id,
            &entity.title,
            entity_type,
            None,
            &config.consolidation,
        )
    };

    // Sync already records the creation event via files::events::record_create_event.
    // We only need to record an additional event if dedup flagged something.
    if dedup.dedup_flagged {
        let conn = storage.conn();
        let payload = json!({"dedup_flagged": true});
        let _ = events::record_event(
            &conn,
            events::EventType::KnowledgeCreated,
            Some(&entity.id),
            &payload,
        );
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
        std::path::PathBuf::from(file_path)
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
    if new_path != abs_path {
        let _ = std::fs::remove_file(&abs_path);
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

    // 6. Record update event
    {
        let conn = storage.conn();
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

/// Build a query response JSON from a list of entities.
pub fn query_response(entities: Vec<crud::Entity>, key: &str) -> serde_json::Value {
    let items: Vec<_> = entities
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "title": e.title,
                "content": e.content,
                "status": e.status,
                "source": e.properties.get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent"),
                "created_at": e.created_at,
                "updated_at": e.updated_at,
                "file_path": e.file_path,
            })
        })
        .collect();
    json!({ key: items, "count": items.len() })
}

/// Build a search response from SearchResults.
pub fn search_response(results: SearchResults) -> serde_json::Value {
    let items: Vec<_> = results
        .results
        .iter()
        .map(|r| {
            json!({
                "id": r.entity.id,
                "type": r.entity.r#type,
                "title": r.entity.title,
                "content": r.entity.content,
                "relevance": r.relevance,
                "graph_path": r.graph_path,
                "graph_path_description": r.graph_path_description,
            })
        })
        .collect();
    json!({
        "results": items,
        "count": items.len(),
        "search_mode": results.search_mode.as_str(),
    })
}

/// Build a context pack response.
pub fn context_response(pack: crate::context::ContextPack) -> serde_json::Value {
    let sections: Vec<_> = pack
        .sections
        .iter()
        .map(|s| {
            json!({
                "source": s.source,
                "entity_id": s.entity_id,
                "title": s.title,
                "content": s.content,
                "relevance": s.relevance,
                "graph_path": s.graph_path,
            })
        })
        .collect();
    json!({
        "query": pack.query,
        "mode": pack.mode.as_str(),
        "sections": sections,
        "metadata": {
            "size_tokens": pack.metadata.size_tokens,
            "selected_sources": pack.metadata.selected_sources,
            "dropped_sources": pack.metadata.dropped_sources,
            "search_mode": pack.metadata.search_mode,
        },
    })
}

// ─── MCP Error Helpers ────────────────────────────────────────────

pub fn mcp_error(code: &str, message: &str) -> McpError {
    McpError::new(
        ErrorCode::INVALID_PARAMS,
        message.to_string(),
        Some(json!({ "code": code })),
    )
}

pub fn mcp_internal_error(context: &str, message: &str) -> McpError {
    McpError::new(
        ErrorCode::INTERNAL_ERROR,
        format!("{}: {}", context, message),
        None,
    )
}

pub fn mcp_invalid_parameter(message: &str) -> McpError {
    McpError::new(ErrorCode::INVALID_PARAMS, message.to_string(), None)
}

/// Embed a query string for hybrid search. Returns None if the
/// embedding model is unavailable (graceful degradation to FTS-only).
#[allow(dead_code)]
pub fn embed_query_for_search(config: &Config, query: &str) -> Option<Vec<f32>> {
    use crate::embed::{EmbeddingModel, ModelType, OnnxEmbeddingModel};

    let models_dir = crate::embed::models_dir();
    let model = OnnxEmbeddingModel::new(
        ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
    );
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

/// Build a successful tool result from a JSON value.
#[allow(dead_code)]
pub fn tool_success(value: serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}
