//! CLI handler for `cogz capture-event`.
//!
//! Reads the event type and optional prompt/tool fields, dispatches
//! to the lifecycle handler, and prints the context pack (if any)
//! to stdout for the agent to consume as injected context.
//!
//! When `hook_json` is true, output is wrapped in the JSON format
//! expected by agent hook systems (Claude Code / Devin compatible):
//! `{"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}`.
//! If no `.cogz/` directory is found, prints `{}` and exits silently.

use std::path::Path;
use std::sync::Arc;

use crate::embed::{ModelType, NliModel, OnnxEmbeddingModel, OnnxNliModel};
use crate::hooks::lifecycle::{
    LifecycleError, LifecycleEvent, LifecycleInput, handle_lifecycle_event,
};
use crate::storage::Storage;

/// Error during capture-event execution.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("lifecycle error: {0}")]
    Lifecycle(#[from] LifecycleError),
    #[error("config error: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
}

/// Result of a capture-event call.
pub struct CaptureResult {
    pub event_id: i64,
    pub observation_id: Option<String>,
    pub context_pack: Option<crate::context::ContextPack>,
    pub reindex_summary: Option<crate::hooks::lifecycle::ReindexSummary>,
    pub consolidation_summary: Option<crate::hooks::lifecycle::ConsolidationSummary>,
}

/// Input parameters for `capture-event`, grouping the optional fields
/// to keep the function signature under clippy's argument limit.
pub struct CaptureInput<'a> {
    pub repo: &'a Path,
    pub event_str: &'a str,
    pub prompt: Option<&'a str>,
    pub prompt_file: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub tool_result: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub hook_json: bool,
    pub fts_only: bool,
}

/// Run `cogz capture-event` — the CLI entry point for hook integration.
///
/// When `hook_json` is true:
/// - If no `.cogz/` is found, prints `{}` and returns (silent skip).
/// - Context packs are wrapped in `hookSpecificOutput.additionalContext`.
/// - Events without context packs print `{}`.
/// - Stdin is parsed for hook payload fields (prompt, tool_name,
///   tool_response, etc.) when the corresponding CLI args are not set.
///
/// When `fts_only` is true:
/// - Skips ONNX model creation entirely (no model loading overhead).
/// - Context assembly uses FTS-only search (fast, ~2s instead of ~40s).
pub fn run_capture_event(input: &CaptureInput) -> Result<CaptureResult, CaptureError> {
    let event = LifecycleEvent::parse(input.event_str).ok_or_else(|| {
        CaptureError::Config(crate::config::ConfigError::Validation(format!(
            "invalid event type '{}': expected session_start, prompt_submit, pre_tool_use, post_tool_use, file_save, session_end, or stop",
            input.event_str
        )))
    })?;

    // When --hook-json is set, read stdin for hook payload fields.
    // Agent hook systems (Devin, Claude Code) pass event data as JSON
    // on stdin. We extract fields that weren't provided via CLI args.
    let (stdin_prompt, stdin_tool_name, stdin_tool_result, stdin_file_path) = if input.hook_json {
        parse_hook_stdin()
    } else {
        (None, None, None, None)
    };

    let prompt = input.prompt.or(stdin_prompt.as_deref());
    let tool_name = input.tool_name.or(stdin_tool_name.as_deref());
    let tool_result = input.tool_result.or(stdin_tool_result.as_deref());
    let file_path = input.file_path.or(stdin_file_path.as_deref());

    let cogz_dir = input.repo.join(".cogz");
    let config_path = cogz_dir.join("config.toml");

    // Silent skip for hook integrations when no .cogz/ exists.
    if !config_path.exists() {
        if input.hook_json {
            println!("{{}}");
            return Ok(CaptureResult {
                event_id: 0,
                observation_id: None,
                context_pack: None,
                reindex_summary: None,
                consolidation_summary: None,
            });
        }
        return Err(CaptureError::Config(
            crate::config::ConfigError::Validation(format!(
                "No .cogz/ directory found in {}. Run `cogz init` first.",
                input.repo.display()
            )),
        ));
    }

    let config = crate::config::load(&config_path)?;
    let db_path = input.repo.join(&config.storage.db_path);

    if !db_path.exists() {
        if input.hook_json {
            println!("{{}}");
            return Ok(CaptureResult {
                event_id: 0,
                observation_id: None,
                context_pack: None,
                reindex_summary: None,
                consolidation_summary: None,
            });
        }
        return Err(CaptureError::Config(
            crate::config::ConfigError::Validation(format!(
                "Database not found at {}. Run `cogz index` first.",
                db_path.display()
            )),
        ));
    }

    let storage = Arc::new(Storage::open(&db_path, config.embedding.dimension)?);

    // Read prompt from file if specified, otherwise use inline prompt.
    let prompt_text = match input.prompt_file {
        Some(path) => Some(std::fs::read_to_string(path).map_err(|e| {
            CaptureError::Config(crate::config::ConfigError::Validation(format!(
                "failed to read prompt file {}: {}",
                path, e
            )))
        })?),
        None => prompt.map(|s| s.to_string()),
    };

    // Create embedding models unless FTS-only mode is requested.
    // FTS-only mode skips ONNX runtime initialization entirely,
    // making hook calls fast (~2s vs ~40s with model loading).
    // Models are lazy-loaded — creating the objects is cheap; the
    // actual ONNX runtime loads on first embed call.
    let (query_model, code_model, nli_model) = if input.fts_only {
        (
            OnnxEmbeddingModel::unavailable(),
            OnnxEmbeddingModel::unavailable(),
            None,
        )
    } else {
        let models_dir = crate::embed::models_dir();
        let nli = OnnxNliModel::with_resource_config(
            &models_dir,
            &config.embedding.nli_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        );
        (
            OnnxEmbeddingModel::with_resource_config(
                ModelType::Knowledge,
                &models_dir,
                config.embedding.dimension,
                &config.embedding.knowledge_model,
                config.embedding.model_idle_ttl,
                config.embedding.model_min_free_mb,
            ),
            OnnxEmbeddingModel::with_resource_config(
                ModelType::Code,
                &models_dir,
                config.embedding.dimension,
                &config.embedding.code_model,
                config.embedding.model_idle_ttl,
                config.embedding.model_min_free_mb,
            ),
            Some(nli),
        )
    };
    let nli_ref: Option<&dyn NliModel> = nli_model.as_ref().map(|m| m as &dyn NliModel);

    let lifecycle_input = LifecycleInput {
        event,
        prompt: prompt_text.as_deref(),
        tool_name,
        tool_result,
        file_path,
    };

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &query_model,
        &code_model,
        nli_ref,
        &lifecycle_input,
    )?;

    // Print output in the requested format.
    if input.hook_json {
        print_hook_json(&event, output.context_pack.as_ref());
    } else if let Some(ref pack) = output.context_pack {
        print_context_pack(pack);
    }

    Ok(CaptureResult {
        event_id: output.event_id,
        observation_id: output.observation_id,
        context_pack: output.context_pack,
        reindex_summary: output.reindex_summary,
        consolidation_summary: output.consolidation_summary,
    })
}

/// Parse stdin for agent hook payload fields. Agent hook systems
/// (Devin, Claude Code) pass event data as JSON on stdin. Extracts:
/// - `prompt` (UserPromptSubmit)
/// - `tool_name` (PreToolUse, PostToolUse, PermissionRequest)
/// - `tool_response.output` or `tool_response.error` (PostToolUse)
/// - `tool_input.file_path` (PostToolUse for edit/write tools)
///
/// Returns (None, None, None, None) if stdin is empty or not valid JSON.
fn parse_hook_stdin() -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    use std::io::Read;

    // Cap stdin at 1MB to avoid memory pressure from large tool outputs
    // on resource-constrained hardware. Tool results larger than this
    // are truncated — the event is still recorded with the truncated payload.
    const MAX_STDIN_BYTES: usize = 1024 * 1024;
    let mut stdin = String::new();
    let mut stdin_buf = std::io::stdin();
    // Read in chunks up to the cap, then drain the rest.
    let mut buf = vec![0u8; 8192];
    loop {
        let n = match stdin_buf.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if stdin.len() + n > MAX_STDIN_BYTES {
            let remaining = MAX_STDIN_BYTES - stdin.len();
            stdin.push_str(&String::from_utf8_lossy(&buf[..remaining]));
            // Drain remaining stdin so the pipe doesn't break.
            let _ = stdin_buf.read_to_end(&mut Vec::new());
            break;
        }
        stdin.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    if stdin.is_empty() {
        return (None, None, None, None);
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdin) else {
        return (None, None, None, None);
    };

    let prompt = json
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tool_name = json
        .get("tool_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // PostToolUse: tool_response has { success, output, error }
    let tool_result = json
        .get("tool_response")
        .and_then(|v| {
            v.get("output")
                .and_then(|o| o.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string())
                })
        })
        .or_else(|| {
            json.get("tool_result")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    // file_save: extract from tool_input.file_path (edit/write tools)
    let file_path = json
        .get("tool_input")
        .and_then(|v| v.get("file_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            json.get("file_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    (prompt, tool_name, tool_result, file_path)
}

/// Print a hook-compatible JSON response. If a context pack was
/// produced, it's wrapped in `hookSpecificOutput.additionalContext`.
/// Otherwise, prints `{}` (no action).
fn print_hook_json(event: &LifecycleEvent, pack: Option<&crate::context::ContextPack>) {
    if let Some(pack) = pack {
        let markdown = format_context_pack(pack);
        let event_name = match event {
            LifecycleEvent::SessionStart => "SessionStart",
            LifecycleEvent::PromptSubmit => "UserPromptSubmit",
            LifecycleEvent::PostToolUse => "PostToolUse",
            LifecycleEvent::SessionEnd => "SessionEnd",
            LifecycleEvent::Stop => "Stop",
            LifecycleEvent::PreToolUse => "PreToolUse",
            LifecycleEvent::FileSave => "PostToolUse",
        };
        let json = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": event_name,
                "additionalContext": markdown,
            }
        });
        println!(
            "{}",
            serde_json::to_string(&json).unwrap_or_else(|_| "{}".into())
        );
    } else {
        println!("{{}}");
    }
}

/// Format a context pack as markdown text for agent injection.
fn format_context_pack(pack: &crate::context::ContextPack) -> String {
    let mut out = String::new();

    for (i, section) in pack.sections.iter().enumerate() {
        let relevance = if section.relevance > 0.0 {
            format!("{:.4}", section.relevance)
        } else {
            "—".to_string()
        };

        out.push_str(&format!(
            "## {}. [{}] {} (relevance: {})\n\n",
            i + 1,
            section.source,
            section.title,
            relevance
        ));

        if section.graph_path.len() > 1 {
            if !section.graph_path_description.is_empty() {
                out.push_str(&format!(
                    "  graph path: {}\n\n",
                    section.graph_path_description
                ));
            } else {
                out.push_str(&format!(
                    "  graph path: {} -> {}\n\n",
                    section.graph_path.first().unwrap_or(&section.entity_id),
                    section.entity_id
                ));
            }
        }

        out.push_str(&section.content);
        out.push_str("\n\n");
    }

    out
}

/// Print a context pack in a format suitable for CLI/human consumption.
fn print_context_pack(pack: &crate::context::ContextPack) {
    if !pack.metadata.dropped_sources.is_empty() {
        eprintln!(
            "Dropped: {} sections over token budget",
            pack.metadata.dropped_sources.len()
        );
    }

    let markdown = format_context_pack(pack);
    print!("{}", markdown);
}
