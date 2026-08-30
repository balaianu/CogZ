//! `update_knowledge` helper — extracted from `helpers.rs` to keep
//! that file under the 400-line structural limit.

use std::sync::Arc;

use rmcp::ErrorData as McpError;
use serde_json::json;

use crate::embed::OnnxEmbeddingModel;
use crate::files::embed_sync::store_embeddings;
use crate::files::frontmatter::FmValue;
use crate::files::{read_entity_file, sync_incremental, write_entity_file};
use crate::mcp::errors::mcp_error;
use crate::mcp::helpers::embed_entity_text;
use crate::mcp::params::UpdateKnowledgeParams;
use crate::storage::{Storage, crud, events};

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
