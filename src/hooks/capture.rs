//! CLI handler for `cogz capture-event`.
//!
//! Reads the event type and optional prompt/tool fields, dispatches
//! to the lifecycle handler, and prints the context pack (if any)
//! to stdout for the agent to consume as injected context.

use std::path::Path;
use std::sync::Arc;

use crate::embed::{ModelType, OnnxEmbeddingModel};
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

/// Run `cogz capture-event` — the CLI entry point for hook scripts.
///
/// Reads the prompt from `prompt` or `prompt_file` (file takes
/// precedence). Dispatches to the lifecycle handler. Prints the
/// context pack sections to stdout if one was produced.
pub fn run_capture_event(
    repo: &Path,
    event_str: &str,
    prompt: Option<&str>,
    prompt_file: Option<&str>,
    tool_name: Option<&str>,
    tool_result: Option<&str>,
    file_path: Option<&str>,
) -> Result<CaptureResult, CaptureError> {
    let event = LifecycleEvent::parse(event_str).ok_or_else(|| {
        CaptureError::Config(crate::config::ConfigError::Validation(format!(
            "invalid event type '{}': expected session_start, prompt_submit, pre_tool_use, post_tool_use, file_save, or session_end",
            event_str
        )))
    })?;

    let cogz_dir = repo.join(".cogz");
    let config_path = cogz_dir.join("config.toml");

    if !config_path.exists() {
        return Err(CaptureError::Config(
            crate::config::ConfigError::Validation(format!(
                "No .cogz/ directory found in {}. Run `cogz init` first.",
                repo.display()
            )),
        ));
    }

    let config = crate::config::load(&config_path)?;
    let db_path = repo.join(&config.storage.db_path);

    if !db_path.exists() {
        return Err(CaptureError::Config(
            crate::config::ConfigError::Validation(format!(
                "Database not found at {}. Run `cogz index` first.",
                db_path.display()
            )),
        ));
    }

    let storage = Arc::new(Storage::open(&db_path, config.embedding.dimension)?);

    // Read prompt from file if specified, otherwise use inline prompt.
    let prompt_text = match prompt_file {
        Some(path) => Some(std::fs::read_to_string(path).map_err(|e| {
            CaptureError::Config(crate::config::ConfigError::Validation(format!(
                "failed to read prompt file {}: {}",
                path, e
            )))
        })?),
        None => prompt.map(|s| s.to_string()),
    };

    let models_dir = crate::embed::models_dir();
    let query_model = OnnxEmbeddingModel::with_model_id(
        ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.knowledge_model,
    );

    let input = LifecycleInput {
        event,
        prompt: prompt_text.as_deref(),
        tool_name,
        tool_result,
        file_path,
    };

    let output = handle_lifecycle_event(&storage, &config, &cogz_dir, &query_model, &input)?;

    // Print context pack to stdout for agent injection.
    if let Some(ref pack) = output.context_pack {
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

/// Print a context pack in a format suitable for agent injection.
/// No human-readable summary header — just the sections.
fn print_context_pack(pack: &crate::context::ContextPack) {
    if !pack.metadata.dropped_sources.is_empty() {
        eprintln!(
            "Dropped: {} sections over token budget",
            pack.metadata.dropped_sources.len()
        );
    }

    for (i, section) in pack.sections.iter().enumerate() {
        let relevance = if section.relevance > 0.0 {
            format!("{:.4}", section.relevance)
        } else {
            "—".to_string()
        };

        println!(
            "## {}. [{}] {} (relevance: {})\n",
            i + 1,
            section.source,
            section.title,
            relevance
        );

        if section.graph_path.len() > 1 {
            if !section.graph_path_description.is_empty() {
                println!("  graph path: {}\n", section.graph_path_description);
            } else {
                println!(
                    "  graph path: {} -> {}\n",
                    section.graph_path.first().unwrap_or(&section.entity_id),
                    section.entity_id
                );
            }
        }

        println!("{}\n", section.content);
    }
}
