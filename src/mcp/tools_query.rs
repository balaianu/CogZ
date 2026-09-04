//! Query tools — query_observations, query_rules, query_knowledge,
//! list_entities.
//!
//! All read-only. Each acquires the DB connection inside
//! spawn_blocking, delegates to a helper for the actual query, and
//! builds the response via `query_response` or `list_entities_response`.

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult};

use crate::mcp::helpers::{
    list_entities_response, mcp_internal_error, query_by_type_with_refs, query_knowledge_with_refs,
};
use crate::mcp::params::*;
use crate::mcp::responses::{query_response, tool_success};
use crate::mcp::server::CogzServer;

pub async fn query_observations(
    server: &CogzServer,
    Parameters(params): Parameters<QueryObservationsParams>,
) -> Result<CallToolResult, McpError> {
    let repo = server.resolve_repo(params.repo.as_deref())?;
    let storage = repo.storage.clone();

    let result = tokio::task::spawn_blocking(move || {
        let conn = storage.conn();
        query_by_type_with_refs(
            &conn,
            "observation",
            params.status.as_deref(),
            params.limit.unwrap_or(20),
            params.references.as_deref(),
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

    let (entities, refs_map) = result;
    Ok(tool_success(query_response(
        entities,
        "observations",
        &refs_map,
    )))
}

pub async fn query_rules(
    server: &CogzServer,
    Parameters(params): Parameters<QueryRulesParams>,
) -> Result<CallToolResult, McpError> {
    let repo = server.resolve_repo(params.repo.as_deref())?;
    let storage = repo.storage.clone();

    let result = tokio::task::spawn_blocking(move || {
        let conn = storage.conn();
        query_by_type_with_refs(
            &conn,
            "rule",
            params.status.as_deref(),
            params.limit.unwrap_or(20),
            params.references.as_deref(),
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

    let (entities, refs_map) = result;
    Ok(tool_success(query_response(entities, "rules", &refs_map)))
}

pub async fn query_knowledge(
    server: &CogzServer,
    Parameters(params): Parameters<QueryKnowledgeParams>,
) -> Result<CallToolResult, McpError> {
    let repo = server.resolve_repo(params.repo.as_deref())?;
    let storage = repo.storage.clone();

    let result = tokio::task::spawn_blocking(move || {
        let conn = storage.conn();
        query_knowledge_with_refs(
            &conn,
            params.status.as_deref(),
            params.category.as_deref(),
            params.tags.as_deref(),
            params.limit.unwrap_or(20),
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

    let (entities, refs_map) = result;
    Ok(tool_success(query_response(
        entities,
        "knowledge",
        &refs_map,
    )))
}

pub async fn list_entities(
    server: &CogzServer,
    Parameters(params): Parameters<ListEntitiesParams>,
) -> Result<CallToolResult, McpError> {
    let repo = server.resolve_repo(params.repo.as_deref())?;
    let storage = repo.storage.clone();
    let entity_type = params.entity_type.clone();
    let status = params.status.clone();

    let result = tokio::task::spawn_blocking(move || {
        let conn = storage.conn();
        list_entities_response(&conn, &entity_type, status.as_deref())
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

    Ok(tool_success(result))
}
