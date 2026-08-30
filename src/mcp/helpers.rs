//! Helper functions for MCP tool handlers.
//!
//! File-first write logic, response builders, and error helpers.
//! Separated from `tools.rs` to keep file sizes under 400 lines.

use std::sync::Arc;

use rmcp::ErrorData as McpError;
use serde_json::json;

use crate::config::Config;
use crate::consolidate::contradict::{
    classify_candidates, fetch_contradiction_candidates, record_contradictions,
};
use crate::embed::{EmbeddingModel, NliModel, OnnxEmbeddingModel};
use crate::files::embed_sync::store_embeddings;
use crate::files::{EntityFile, sync_incremental, write_entity_file};
use crate::mcp::dedup::check_duplicate;
use crate::storage::crud::Entity;
use crate::storage::{Storage, events, query};

// Re-export error helpers and response builders for backward-compatible imports.
pub use crate::mcp::errors::{mcp_error, mcp_internal_error, mcp_invalid_parameter};
pub use crate::mcp::responses::{context_response, query_response, search_response, tool_success};
// Re-export status builder so tools.rs imports unchanged.
pub use crate::mcp::status::build_status_response;

/// Default status filter for query tools: "active" when no status is
/// provided, matching the MCP contract.
pub const DEFAULT_QUERY_STATUS: &str = "active";

/// Type alias for the query result: entities with their references map.
pub type QueryWithRefs = (Vec<Entity>, std::collections::HashMap<String, Vec<String>>);

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
pub fn create_entity_file(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &std::path::Path,
    entity: &EntityFile,
    embed_model: Option<&OnnxEmbeddingModel>,
    nli_model: Option<&dyn NliModel>,
) -> Result<serde_json::Value, McpError> {
    write_and_sync(
        storage,
        config,
        cogz_dir,
        entity,
        entity.entity_type.as_str(),
        embed_model,
        nli_model,
    )
}

/// Write an entity file and sync it to the DB. Returns the tool
/// response JSON with id, file_path, status, dedup results, and
/// contradiction flag.
pub fn write_and_sync(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &std::path::Path,
    entity: &EntityFile,
    entity_type: &str,
    embed_model: Option<&OnnxEmbeddingModel>,
    nli_model: Option<&dyn NliModel>,
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

    // 5. Contradiction check — NLI inference is blocking I/O, so it
    // must not hold the DB mutex. Fetch candidates under the lock,
    // drop the lock, run NLI, then re-acquire to record results.
    let contradiction_flagged = if matches!(
        entity.entity_type,
        crate::files::FileEntityType::Observation | crate::files::FileEntityType::Rule
    ) {
        let candidates = {
            let conn = storage.conn();
            fetch_contradiction_candidates(
                &conn,
                &entity.id,
                &entity.body,
                entity_type,
                &config.consolidation,
            )
        };

        // NLI classification outside the lock.
        let contradicts_ids = classify_candidates(candidates, &entity.body, nli_model);

        if !contradicts_ids.is_empty() {
            let conn = storage.conn();
            if let Err(e) = record_contradictions(&conn, &entity.id, &contradicts_ids, &path) {
                tracing::warn!("failed to record contradictions for {}: {}", entity.id, e);
            }
            true
        } else {
            false
        }
    } else {
        false
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
        "contradiction_flagged": contradiction_flagged,
        "duplicate_warning": dedup.duplicate_warning,
    }))
}

/// Update a knowledge file in-place: read existing, modify, write, sync.
/// Implementation in `update_knowledge.rs`.
pub use crate::mcp::update_knowledge::update_knowledge_file;

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
    if mode.requires_query() && query.is_none() {
        return Err(mcp_invalid_parameter(&format!(
            "query is required for {} mode",
            mode
        )));
    }
    Ok(mode)
}
