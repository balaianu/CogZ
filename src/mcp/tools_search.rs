//! Search and context tools — search, get_context.
//!
//! Both tools embed the query (when the model is available) and
//! delegate to the search/context layers. The embedding happens
//! inside spawn_blocking to avoid holding the DB mutex during ONNX
//! inference.

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult};

use crate::context::{AssembleParams, assemble_context};
use crate::mcp::helpers::{embed_query_for_search, mcp_internal_error, parse_context_mode};
use crate::mcp::params::*;
use crate::mcp::responses::{context_response, search_response, tool_success};
use crate::mcp::server::CogzServer;
use crate::search::{SearchParams, search as search_entities};

pub async fn search(
    server: &CogzServer,
    Parameters(params): Parameters<SearchToolParams>,
) -> Result<CallToolResult, McpError> {
    let storage = server.storage.clone();
    let search_config = server.config.search.clone();
    let default_limit = server.config.search.max_results;
    let task_max_hops = server.config.context.task_max_hops;
    let query_model = server.query_model.clone();
    let code_model = server.code_model.clone();
    let use_code = params.code_search.unwrap_or(false);

    let results = tokio::task::spawn_blocking(move || {
        let model = if use_code { &code_model } else { &query_model };
        let query_embedding = embed_query_for_search(model, &params.query);
        let conn = storage.conn();
        let expand = params.expand.unwrap_or(true);
        let search_params = SearchParams {
            entity_type: params.entity_type,
            status: params.status,
            limit: params.limit.unwrap_or(default_limit),
            expand,
            max_hops: if expand { task_max_hops } else { 0 },
        };
        search_entities(
            &conn,
            &params.query,
            query_embedding.as_deref(),
            &search_params,
            &search_config,
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("search", &e.to_string()))?;

    Ok(tool_success(search_response(results)))
}

pub async fn get_context(
    server: &CogzServer,
    Parameters(params): Parameters<GetContextParams>,
) -> Result<CallToolResult, McpError> {
    let mode_str = params.mode.as_deref().unwrap_or("task");
    let mode = parse_context_mode(mode_str, params.query.as_deref())?;
    let query_str = params.query.clone();
    let storage = server.storage.clone();
    let config = server.config.clone();
    let query_model = server.query_model.clone();

    let pack = tokio::task::spawn_blocking(move || {
        let query_embedding = params
            .query
            .as_deref()
            .and_then(|q| embed_query_for_search(&query_model, q));
        let conn = storage.conn();
        assemble_context(
            &conn,
            &AssembleParams {
                mode,
                query: query_str.as_deref(),
                query_embedding: query_embedding.as_deref(),
                max_tokens: params.max_tokens,
                include_stale: params.include_stale.unwrap_or(false),
            },
            &config,
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("context", &e.to_string()))?;

    Ok(tool_success(context_response(pack)))
}
