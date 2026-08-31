//! Write tools — record_observation, create_rule, create_knowledge,
//! update_knowledge.
//!
//! All write tools follow the file-first invariant: the entity file
//! is written before the DB is touched. The actual write-and-sync
//! logic lives in `helpers.rs` and `update_knowledge.rs`.

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult};
use serde_json::json;

use crate::files::frontmatter::FmValue;
use crate::files::{EntityFile, FileEntityType};
use crate::mcp::helpers::{
    auto_title, create_entity_file, mcp_internal_error, update_knowledge_file, write_and_sync,
};
use crate::mcp::params::*;
use crate::mcp::responses::tool_success;
use crate::mcp::server::CogzServer;

pub async fn record_observation(
    server: &CogzServer,
    Parameters(params): Parameters<RecordObservationParams>,
) -> Result<CallToolResult, McpError> {
    let title = params.title.unwrap_or_else(|| auto_title(&params.content));
    let storage = server.storage.clone();
    let config = server.config.clone();
    let cogz_dir = server.cogz_dir.clone();
    let query_model = server.query_model.clone();
    let nli_model = server.nli_model.clone();

    let result = tokio::task::spawn_blocking(move || {
        let mut entity = EntityFile::new(&title, FileEntityType::Observation, &params.content);

        if let Some(refs) = params.references.as_deref() {
            entity.references = refs.to_vec();
        }

        let source = params.source.unwrap_or_else(|| "agent".to_string());
        entity.frontmatter.insert("source", FmValue::String(source));
        entity.frontmatter.insert("confidence", FmValue::Float(0.5));

        create_entity_file(
            &storage,
            &config,
            &cogz_dir,
            &entity,
            Some(&query_model),
            Some(&*nli_model),
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

    Ok(tool_success(json!(result)))
}

pub async fn create_rule(
    server: &CogzServer,
    Parameters(params): Parameters<CreateRuleParams>,
) -> Result<CallToolResult, McpError> {
    let title = params.title.unwrap_or_else(|| auto_title(&params.content));
    let storage = server.storage.clone();
    let config = server.config.clone();
    let cogz_dir = server.cogz_dir.clone();
    let query_model = server.query_model.clone();
    let nli_model = server.nli_model.clone();

    let result = tokio::task::spawn_blocking(move || {
        let mut entity = EntityFile::new(&title, FileEntityType::Rule, &params.content);

        if let Some(refs) = params.references.as_deref() {
            entity.references = refs.to_vec();
        }

        let confidence = params.confidence.unwrap_or(1.0);
        entity
            .frontmatter
            .insert("confidence", FmValue::Float(confidence));

        create_entity_file(
            &storage,
            &config,
            &cogz_dir,
            &entity,
            Some(&query_model),
            Some(&*nli_model),
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

    Ok(tool_success(json!(result)))
}

pub async fn create_knowledge(
    server: &CogzServer,
    Parameters(params): Parameters<CreateKnowledgeParams>,
) -> Result<CallToolResult, McpError> {
    let storage = server.storage.clone();
    let config = server.config.clone();
    let cogz_dir = server.cogz_dir.clone();
    let query_model = server.query_model.clone();

    let result = tokio::task::spawn_blocking(move || {
        let mut entity = EntityFile::new(&params.title, FileEntityType::Knowledge, &params.content);

        entity
            .frontmatter
            .insert("category", FmValue::String(params.category.clone()));

        if let Some(tags) = &params.tags {
            entity
                .frontmatter
                .insert("tags", FmValue::Array(tags.clone()));
        }

        if let Some(refs) = &params.references {
            entity.references = refs.clone();
        }

        write_and_sync(
            &storage,
            &config,
            &cogz_dir,
            &entity,
            "knowledge",
            Some(&query_model),
            None,
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

    Ok(tool_success(json!(result)))
}

pub async fn update_knowledge(
    server: &CogzServer,
    Parameters(params): Parameters<UpdateKnowledgeParams>,
) -> Result<CallToolResult, McpError> {
    let storage = server.storage.clone();
    let cogz_dir = server.cogz_dir.clone();
    let query_model = server.query_model.clone();

    let result = tokio::task::spawn_blocking(move || {
        update_knowledge_file(&storage, &cogz_dir, &params, Some(&query_model))
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

    Ok(tool_success(json!(result)))
}
