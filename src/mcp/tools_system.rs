//! System tools — get_status, consolidate, capture_event.
//!
//! `get_status` reports DB stats and model availability.
//! `consolidate` runs the deferred promotion and merge phases.
//! `capture_event` handles lifecycle events from hook scripts.

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult};
use serde_json::json;

use crate::hooks::lifecycle::{LifecycleEvent, LifecycleInput, handle_lifecycle_event};
use crate::mcp::helpers::{build_status_response, mcp_internal_error};
use crate::mcp::params::{CaptureEventParams, ConsolidateParams, GetStatusParams};
use crate::mcp::responses::tool_success;
use crate::mcp::server::CogzServer;

pub async fn get_status(
    server: &CogzServer,
    Parameters(params): Parameters<GetStatusParams>,
) -> Result<CallToolResult, McpError> {
    let repo = server.resolve_repo(&params.repo)?;
    let storage = repo.storage.clone();
    let config = repo.config.clone();

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
    let repo = server.resolve_repo(&params.repo)?;
    let storage = repo.storage.clone();
    let config = repo.config.clone();
    let cogz_dir = repo.cogz_dir.clone();
    let nli_model = repo.nli_model.clone();
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
            Some(&*nli_model),
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

pub async fn capture_event(
    server: &CogzServer,
    Parameters(params): Parameters<CaptureEventParams>,
) -> Result<CallToolResult, McpError> {
    let event = LifecycleEvent::parse(&params.event_type).ok_or_else(|| {
        mcp_internal_error(
            "capture_event",
            &format!(
                "invalid event_type '{}': expected session_start, prompt_submit, pre_tool_use, post_tool_use, file_save, session_end, or stop",
                params.event_type
            ),
        )
    })?;

    let repo = server.resolve_repo(&params.repo)?;
    let storage = repo.storage.clone();
    let config = repo.config.clone();
    let cogz_dir = repo.cogz_dir.clone();
    let query_model = repo.query_model.clone();
    let code_model = repo.code_model.clone();
    let nli_model = repo.nli_model.clone();
    let prompt = params.prompt.clone();
    let tool_name = params.tool_name.clone();
    let tool_result = params.tool_result.clone();
    let file_path = params.file_path.clone();

    let result = tokio::task::spawn_blocking(move || {
        let input = LifecycleInput {
            event,
            prompt: prompt.as_deref(),
            tool_name: tool_name.as_deref(),
            tool_result: tool_result.as_deref(),
            file_path: file_path.as_deref(),
        };
        handle_lifecycle_event(
            &storage,
            &config,
            &cogz_dir,
            &query_model,
            &code_model,
            Some(&*nli_model),
            &input,
        )
    })
    .await
    .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
    .map_err(|e| mcp_internal_error("capture_event", &e.to_string()))?;

    let response = json!({
        "event_type": params.event_type,
        "event_id": result.event_id,
        "observation_id": result.observation_id,
        "context_pack": result.context_pack.as_ref().map(|pack| {
            crate::mcp::responses::context_response_ref(pack)
        }),
        "reindex_summary": result.reindex_summary,
        "consolidation_summary": result.consolidation_summary,
    });

    Ok(tool_success(response))
}
