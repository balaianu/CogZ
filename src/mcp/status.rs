//! Status response builder — assembles the `get_status` JSON from
//! DB stats and model availability checks.

use serde_json::json;

use crate::config::Config;
use crate::storage::{Storage, crud, events};

/// Check whether the embedding models are available on disk (without
/// loading them). Returns a JSON object matching the MCP contract:
/// `embedding_code`, `embedding_knowledge`, and `nli` — each with
/// `{available, name}`.
pub fn model_availability(config: &Config) -> serde_json::Value {
    use crate::embed::{ModelType, OnnxEmbeddingModel, OnnxNliModel};

    let models_dir = crate::embed::models_dir();
    let knowledge = OnnxEmbeddingModel::with_model_id(
        ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.knowledge_model,
    );
    let code = OnnxEmbeddingModel::with_model_id(
        ModelType::Code,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.code_model,
    );

    let nli_available = if config.embedding.nli_model.is_empty() {
        false
    } else {
        let nli = OnnxNliModel::new(&models_dir, &config.embedding.nli_model);
        nli.model_files_exist()
    };

    json!({
        "embedding_code": {
            "available": code.model_files_exist(),
            "name": &config.embedding.code_model,
        },
        "embedding_knowledge": {
            "available": knowledge.model_files_exist(),
            "name": &config.embedding.knowledge_model,
        },
        "nli": {
            "available": nli_available,
            "name": if config.embedding.nli_model.is_empty() {
                serde_json::Value::Null
            } else {
                json!(config.embedding.nli_model)
            },
        },
    })
}

/// Build the `get_status` response JSON. Queries DB stats and checks
/// model availability. The DB path is extracted under the lock and
/// filesystem metadata is read outside the lock to avoid blocking
/// other callers.
pub fn build_status_response(
    storage: &Storage,
    config: &Config,
) -> Result<serde_json::Value, crate::storage::StorageError> {
    let db_path = {
        let conn = storage.conn();
        let path: Option<String> = conn
            .query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
            .ok();
        path
    };
    let db_size = match &db_path {
        Some(p) if !p.is_empty() => std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
        _ => 0,
    };

    // Complete all DB queries in a scoped block, then drop the guard
    // before probing model files on disk. This avoids blocking all DB
    // work during filesystem metadata calls.
    let (counts, total, stale, edges, events_count, schema_version, last_index) = {
        let conn = storage.conn();
        (
            crate::storage::query::entity_counts_by_type(&conn)?,
            crud::count_all(&conn)?,
            crate::storage::query::count_stale(&conn)?,
            crate::storage::edges::count_edges(&conn)?,
            events::count_events(&conn)?,
            conn.query_row::<u32, _, _>("PRAGMA user_version", [], |r| r.get(0))?,
            crate::storage::get_meta(&conn, "last_index"),
        )
    };

    // Model availability probes filesystem — no DB lock held.
    let models = model_availability(config);

    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": schema_version,
        "db_path": db_path.unwrap_or_default(),
        "db_size_bytes": db_size,
        "models": models,
        "entities": counts.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "total_entities": total,
        "stale_count": stale,
        "edges": edges,
        "events": events_count,
        "last_index": last_index,
    }))
}
