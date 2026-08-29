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
    use crate::embed::{ModelType, OnnxEmbeddingModel};

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
            "available": false,
            "name": null,
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

    let conn = storage.conn();
    let counts = crate::storage::query::entity_counts_by_type(&conn)?;
    let total = crud::count_all(&conn)?;
    let stale = crate::storage::query::count_stale(&conn)?;
    let edges = crate::storage::edges::count_edges(&conn)?;
    let events_count = events::count_events(&conn)?;
    let schema_version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let last_index = crate::storage::get_meta(&conn, "last_index");

    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": schema_version,
        "db_path": db_path.unwrap_or_default(),
        "db_size_bytes": db_size,
        "models": model_availability(config),
        "entities": counts.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "total_entities": total,
        "stale_count": stale,
        "edges": edges,
        "events": events_count,
        "last_index": last_index,
    }))
}
