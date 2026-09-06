use std::sync::Arc;

use rmcp::ErrorData as McpError;
use serde_json::json;

use crate::config::Config;
use crate::consolidate::contradict::{
    classify_candidates, fetch_contradiction_candidates, record_contradictions,
};
use crate::embed::{NliModel, OnnxEmbeddingModel};
use crate::files::embed_sync::store_embeddings;
use crate::files::{EntityFile, sync_single_file};
use crate::mcp::dedup::check_duplicate;
use crate::mcp::errors::mcp_error;
use crate::mcp::helpers::{embed_entity_text, write_entity_file_atomic};
use crate::storage::{Storage, events};

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
    // 0. Scan for secrets before writing to disk. Knowledge and rules
    // are committed to git — a secret there is nearly impossible to
    // remove from version history.
    if let Some(scan) = crate::security::scan_content(&entity.title, &entity.body) {
        return Err(mcp_error(
            "secret_detected",
            &format!(
                "Content contains a suspected {} (starting with \"{}...\"). \
                 Remove the secret before recording this entity.",
                scan.kind, scan.preview
            ),
        ));
    }

    // 1. Write file first (file-first invariant). Use file_path_safe
    // to avoid silent overwrites when titles slugify identically.
    // Retry with create_new to close the race window between
    // file_path_safe's existence check and the write: two concurrent
    // creates with colliding slugs could both see the path as unused.
    let path = entity.file_path_safe(cogz_dir);
    let path = write_entity_file_atomic(&path, entity, cogz_dir).map_err(|e| {
        mcp_error(
            "file_write_failed",
            &format!("Failed to write entity file: {}", e),
        )
    })?;

    // 2. Sync to DB — only the single new/changed file, not the
    // entire .cogz/ directory. This avoids O(n) filesystem reads on
    // every MCP write call.
    let relative_path = path.strip_prefix(cogz_dir).unwrap_or(&path);
    let sync_result = sync_single_file(storage, cogz_dir, &relative_path.to_string_lossy());
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
            store_embeddings(
                &mut conn,
                &[(entity.id.clone(), entity_type.to_string(), emb.clone())],
            );
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
        let contradicts_ids = classify_candidates(
            candidates,
            &entity.body,
            nli_model,
            embed_model.map(|m| m as &dyn crate::embed::EmbeddingModel),
            &config.consolidation,
        );

        if !contradicts_ids.is_empty() {
            if let Err(e) =
                record_contradictions(storage, cogz_dir, &entity.id, &contradicts_ids, &path)
            {
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

    Ok(json!({
        "id": entity.id,
        "file_path": relative_path.display().to_string(),
        "status": entity.status,
        "dedup_flagged": dedup.dedup_flagged,
        "contradiction_flagged": contradiction_flagged,
        "duplicate_warning": dedup.duplicate_warning,
    }))
}

// ─── Response builders ─────────────────────────────────────────────
// Response builder functions live in `responses.rs` and are re-exported above.
