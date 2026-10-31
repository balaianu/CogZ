//! CLI handler for `cogz capture-event`.
//!
//! Reads the event type and optional prompt/tool fields, dispatches
//! to the lifecycle handler, and prints the context pack (if any)
//! to stdout for the agent to consume as injected context.
//!
//! When `hook_json` is true, output is wrapped in the JSON format
//! expected by agent hook systems (Claude Code / Devin compatible):
//! `{"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}`.
//! If no `.cogz/` directory is found, prints nothing and exits silently.

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
    #[error(
        "invalid event type '{0}': expected session_start, prompt_submit, pre_tool_use, post_tool_use, file_save, session_end, or stop"
    )]
    InvalidEvent(String),
    #[error("failed to read prompt file {path}: {source}")]
    PromptFile {
        path: String,
        source: std::io::Error,
    },
}

/// Result of a capture-event call.
pub struct CaptureResult {
    pub event_id: Option<i64>,
    pub observation_id: Option<String>,
    pub context_pack: Option<crate::context::ContextPack>,
    pub reindex_summary: Option<crate::hooks::lifecycle::ReindexSummary>,
    pub notices: Option<String>,
    pub consolidation_summary: Option<crate::hooks::lifecycle::ConsolidationSummary>,
    pub suggestion_count: Option<usize>,
    /// Text the caller should print to stdout — the hook JSON
    /// envelope, or the rendered pack + notice. Empty when there is
    /// nothing to emit; the caller skips printing, which is how hook
    /// consumers see a no-op. Libraries return strings; binaries
    /// print them.
    pub emission: String,
}

impl CaptureResult {
    /// All-None result for the silent-skip early exits. Emission is
    /// empty: hook consumers treat absent stdout as a no-op, while a
    /// bare `{}` matches no strict output schema (Devin post_compact
    /// rejects it as invalid JSON output).
    fn empty(emission: &str) -> Self {
        Self {
            event_id: None,
            observation_id: None,
            context_pack: None,
            reindex_summary: None,
            notices: None,
            consolidation_summary: None,
            suggestion_count: None,
            emission: emission.to_string(),
        }
    }
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
    pub hook_format: crate::hooks::format::HookFormat,
    /// Record the event and run the lifecycle but emit nothing. For
    /// hook firings whose consumer event cannot inject context
    /// (compact events on most agents) — empty stdout is the only
    /// no-op shape that validates everywhere.
    pub record_only: bool,
}

/// Run `cogz capture-event` — the CLI entry point for hook integration.
/// Returns the result plus `emission`, the text the binary prints to
/// stdout.
///
/// When `hook_json` is true:
/// - If no `.cogz/` is found, emission is empty (silent skip).
/// - Context packs are wrapped in `hookSpecificOutput.additionalContext`.
/// - Events without context packs emit nothing.
/// - Stdin is parsed for hook payload fields (prompt, tool_name,
///   tool_response, etc.) when the corresponding CLI args are not set.
///
/// When `fts_only` is true:
/// - Skips ONNX model creation entirely (no model loading overhead).
/// - Context assembly uses FTS-only search (fast, ~2s instead of ~40s).
pub fn run_capture_event(input: &CaptureInput) -> Result<CaptureResult, CaptureError> {
    let event = LifecycleEvent::parse(input.event_str)
        .ok_or_else(|| CaptureError::InvalidEvent(input.event_str.to_string()))?;

    // When --hook-json is set, read stdin for hook payload fields.
    // Agent hook systems (Devin, Claude Code) pass event data as JSON
    // on stdin. We extract fields that weren't provided via CLI args.
    let stdin = if input.hook_json {
        parse_hook_stdin()
    } else {
        HookStdinFields::default()
    };

    let prompt = input.prompt.or(stdin.prompt.as_deref());
    let tool_name = input.tool_name.or(stdin.tool_name.as_deref());
    let tool_result = input.tool_result.or(stdin.tool_result.as_deref());
    let file_path = input.file_path.or(stdin.file_path.as_deref());

    let cogz_dir = input.repo.join(".cogz");
    let config_path = cogz_dir.join("config.toml");

    // Silent skip for hook integrations when no .cogz/ exists.
    if !config_path.exists() {
        if input.hook_json {
            return Ok(CaptureResult::empty(""));
        }
        return Err(CaptureError::Config(
            crate::config::ConfigError::CogzDirMissing(cogz_dir.clone()),
        ));
    }

    let config = crate::config::load(&config_path)?;
    crate::embed::set_ort_download_allowed(config.embedding.auto_download);
    crate::embed::set_fts_only(config.embedding.force_fts_only);
    crate::embed::set_code_embeddings(config.embedding.code_embeddings);
    crate::storage::search_terms::set_split_identifiers(config.index.split_identifiers);
    let db_path = crate::config::resolve_db_path(input.repo, &config.storage.db_path)?;

    if !db_path.exists() {
        if input.hook_json {
            return Ok(CaptureResult::empty(""));
        }
        return Err(CaptureError::Config(crate::config::ConfigError::DbMissing(
            db_path.clone(),
        )));
    }

    let storage = Arc::new(Storage::open(&db_path, config.embedding.dimension)?);

    // Read prompt from file if specified, otherwise use inline prompt.
    let prompt_text = match input.prompt_file {
        Some(path) => {
            Some(
                std::fs::read_to_string(path).map_err(|e| CaptureError::PromptFile {
                    path: path.to_string(),
                    source: e,
                })?,
            )
        }
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

    // Compose stdout text in the requested format — the caller
    // prints it, so the library stays pure for MCP and tests. A
    // record-only call still runs the lifecycle (event record,
    // consolidation, reindex) but emits nothing: compact firings
    // whose native event cannot inject context would otherwise push
    // an output shape the consumer's schema rejects.
    let emission = if input.record_only {
        String::new()
    } else if input.hook_json {
        crate::hooks::format::hook_json(
            &event,
            output.context_pack.as_ref(),
            file_path,
            output.notices.as_deref(),
            input.hook_format,
            Some(input.repo),
            stdin.hook_event_name.as_deref(),
        )
    } else {
        let mut text = String::new();
        if let Some(ref pack) = output.context_pack {
            text.push_str(&crate::hooks::format::context_pack_markdown(
                pack,
                Some(input.repo),
            ));
        }
        if let Some(ref notice) = output.notices {
            text.push_str(notice);
            text.push('\n');
        }
        text
    };

    Ok(CaptureResult {
        event_id: output.event_id,
        observation_id: output.observation_id,
        context_pack: output.context_pack,
        reindex_summary: output.reindex_summary,
        notices: output.notices,
        consolidation_summary: output.consolidation_summary,
        suggestion_count: output.suggestion_count,
        emission,
    })
}

/// Fields recovered from the agent's hook stdin payload.
#[derive(Default)]
struct HookStdinFields {
    prompt: Option<String>,
    tool_name: Option<String>,
    tool_result: Option<String>,
    file_path: Option<String>,
    /// The consumer's own name for the firing event. Devin, Claude,
    /// Codex, and Gemini all send `hook_event_name` on stdin; echoing
    /// it back in `hookSpecificOutput` keeps the output under the
    /// event's own schema when a native event is aliased to a
    /// different lifecycle event (PostCompaction → session_start)
    /// or uses names we do not map (Gemini's BeforeAgent).
    hook_event_name: Option<String>,
}

/// Parse stdin for agent hook payload fields. Agent hook systems
/// (Devin, Claude Code) pass event data as JSON on stdin. Extracts:
/// - `prompt` (UserPromptSubmit)
/// - `tool_name` (PreToolUse, PostToolUse, PermissionRequest)
/// - `tool_response.output` or `tool_response.error` (PostToolUse)
/// - `tool_input.file_path` (PostToolUse for edit/write tools)
/// - `hook_event_name` (echoed back in hookSpecificOutput)
///
/// Returns the default (all-None) fields if stdin is empty or not
/// valid JSON.
fn parse_hook_stdin() -> HookStdinFields {
    let mut stdin_buf = std::io::stdin();
    let (bytes, capped) = read_stdin_capped(&mut stdin_buf);
    if bytes.is_empty() {
        return HookStdinFields::default();
    }

    let stdin = String::from_utf8_lossy(&bytes);
    parse_hook_payload(&stdin, capped)
}

/// Read a hook payload into a byte buffer capped at 1 MiB. Returns
/// (bytes, capped) — `capped` is true when the input exceeded the
/// cap, in which case the remainder is drained to a sink so the
/// writer's pipe doesn't break. Decoding happens once, at the
/// caller: per-chunk `from_utf8_lossy` corrupts multi-byte
/// characters that span a read boundary.
fn read_stdin_capped(reader: &mut impl std::io::Read) -> (Vec<u8>, bool) {
    const MAX_STDIN_BYTES: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes.len() + n > MAX_STDIN_BYTES {
            let remaining = MAX_STDIN_BYTES - bytes.len();
            bytes.extend_from_slice(&buf[..remaining]);
            let _ = std::io::copy(reader, &mut std::io::sink());
            return (bytes, true);
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    (bytes, false)
}

/// Extract hook fields from a decoded payload. `capped` marks input
/// that was cut at the read cap — a failed parse then falls back to
/// recovering whichever complete string fields survived.
fn parse_hook_payload(raw: &str, capped: bool) -> HookStdinFields {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(raw) else {
        return extract_fields_from_truncated(raw, capped);
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
            // Copilot's snake_case payload sends `tool_result` as an
            // object ({resultType, content/output}); accept a bare
            // string too.
            json.get("tool_result").and_then(|v| {
                v.as_str().map(|s| s.to_string()).or_else(|| {
                    v.get("output")
                        .or_else(|| v.get("content"))
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string())
                })
            })
        })
        .or_else(|| {
            // Cursor sends `tool_output` — a JSON-stringified string.
            json.get("tool_output")
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

    let hook_event_name = json
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    HookStdinFields {
        prompt,
        tool_name,
        tool_result,
        file_path,
        hook_event_name,
    }
}

/// Recover fields from a stdin payload cut at the size cap. The tail
/// — usually `tool_response.output` — ends mid-string, so a full
/// parse fails. Scan for complete JSON string tokens and decode them
/// through serde so escapes unmarshal correctly. Fields that didn't
/// survive whole return None; the event still records prompt, tool
/// name, and file path instead of dropping entirely.
fn extract_fields_from_truncated(raw: &str, capped: bool) -> HookStdinFields {
    if !capped {
        return HookStdinFields::default();
    }
    let prompt = scan_json_string_field(raw, "prompt");
    let tool_name = scan_json_string_field(raw, "tool_name");
    let tool_result = scan_json_string_field(raw, "output")
        .or_else(|| scan_json_string_field(raw, "error"))
        .or_else(|| scan_json_string_field(raw, "content"))
        .or_else(|| scan_json_string_field(raw, "tool_result"))
        .or_else(|| scan_json_string_field(raw, "tool_output"));
    let file_path = raw
        .find("\"tool_input\"")
        .and_then(|anchor| scan_json_string_field(&raw[anchor..], "file_path"))
        .or_else(|| scan_json_string_field(raw, "file_path"));
    let hook_event_name = scan_json_string_field(raw, "hook_event_name");
    HookStdinFields {
        prompt,
        tool_name,
        tool_result,
        file_path,
        hook_event_name,
    }
}

/// Find `"key": "..."` and return the decoded string value, skipping
/// key occurrences whose value is missing, non-string, or truncated.
fn scan_json_string_field(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for (idx, _) in raw.match_indices(&needle) {
        let after = raw[idx + needle.len()..].trim_start();
        let Some(after) = after.strip_prefix(':').map(str::trim_start) else {
            continue;
        };
        if !after.starts_with('"') {
            continue;
        }
        let mut escaped = false;
        for (i, c) in after[1..].char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                return serde_json::from_str::<String>(&after[..i + 2]).ok();
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
