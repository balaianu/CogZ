//! System tools — get_status, consolidate.
//!
//! `get_status` reports DB stats and model availability.
//! `consolidate` runs the deferred promotion and merge phases.

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult};
use serde_json::json;

use crate::mcp::helpers::{build_status_response, mcp_internal_error};
use crate::mcp::params::ConsolidateParams;
use crate::mcp::responses::tool_success;
use crate::mcp::server::CogzServer;

pub async fn get_status(server: &CogzServer) -> Result<CallToolResult, McpError> {
    let storage = server.storage.clone();
    let config = server.config.clone();

    let status = tokio::task::spawn_blocking(move || build_status_response(&storage, &config))
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("status", &e.to_string()))?;

    Ok(tool_success(status))
}

pub async fn consolidate(
    server: &CogzServer,
    Parameters(params): Parameters<ConsolidateParams>,
) -> Result<CallToolResult, McpError> {
    let storage = server.storage.clone();
    let config = server.config.clone();
    let cogz_dir = server.cogz_dir.clone();
    let dry_run = params.dry_run;

    let result = tokio::task::spawn_blocking(move || {
        let promoted = crate::consolidate::promote::run_promotion(
            &storage,
            &cogz_dir,
            &config.consolidation,
            dry_run,
        )
        .map_err(|e| mcp_internal_error("consolidate", &e.to_string()))?;

        let merged = crate::consolidate::merge::run_merge(
            &storage,
            &cogz_dir,
            &config.consolidation,
            dry_run,
        )
        .map_err(|e| mcp_internal_error("consolidate", &e.to_string()))?;

        Ok::<_, McpError>(json!({
            "promoted": promoted,
            "merged": merged,
            "promoted_count": promoted.len(),
            "merged_count": merged.len(),
            "dry_run": dry_run,
        }))
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

    Ok(tool_success(result))
}
