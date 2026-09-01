//! Lifecycle event handlers — session_start, prompt_submit,
//! pre_tool_use, post_tool_use, file_save.
//!
//! Each handler records a domain event. `session_start` and
//! `prompt_submit` also assemble a context pack for injection.
//! `post_tool_use` optionally records an observation when both
//! `tool_name` and `tool_result` are provided. `file_save` triggers
//! an incremental code reindex and stale-knowledge flagging when the
//! saved file is a source file (not under `.cogz/`).

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use serde_json::json;

use crate::config::Config;
use crate::context::{AssembleParams, ContextMode, ContextPack, assemble_context};
use crate::embed::OnnxEmbeddingModel;
use crate::files::frontmatter::FmValue;
use crate::files::sync_incremental;
use crate::files::{EntityFile, FileEntityType, write_entity_file};
use crate::storage::{Storage, events};

/// Which lifecycle event triggered the hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    SessionStart,
    PromptSubmit,
    PreToolUse,
    PostToolUse,
    FileSave,
    SessionEnd,
}

impl LifecycleEvent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session_start" => Some(Self::SessionStart),
            "prompt_submit" => Some(Self::PromptSubmit),
            "pre_tool_use" => Some(Self::PreToolUse),
            "post_tool_use" => Some(Self::PostToolUse),
            "file_save" => Some(Self::FileSave),
            "session_end" => Some(Self::SessionEnd),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::PromptSubmit => "prompt_submit",
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::FileSave => "file_save",
            Self::SessionEnd => "session_end",
        }
    }

    fn event_type(&self) -> events::EventType {
        match self {
            Self::SessionStart => events::EventType::SessionStart,
            Self::PromptSubmit => events::EventType::PromptSubmit,
            Self::PreToolUse => events::EventType::PreToolUse,
            Self::PostToolUse => events::EventType::PostToolUse,
            Self::FileSave => events::EventType::FileSave,
            Self::SessionEnd => events::EventType::SessionEnd,
        }
    }
}

impl std::fmt::Display for LifecycleEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Input parameters for a lifecycle event.
pub struct LifecycleInput<'a> {
    pub event: LifecycleEvent,
    pub prompt: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub tool_result: Option<&'a str>,
    pub file_path: Option<&'a str>,
}

/// Result of handling a lifecycle event.
pub struct LifecycleOutput {
    pub event_id: i64,
    /// Context pack for session_start and prompt_submit; None for tool events.
    pub context_pack: Option<ContextPack>,
    /// Observation UUID if an observation was recorded (post_tool_use only).
    pub observation_id: Option<String>,
    /// Reindex summary if a file_save triggered code reindexing.
    pub reindex_summary: Option<ReindexSummary>,
    /// Consolidation dry-run summary if session_end triggered it.
    pub consolidation_summary: Option<ConsolidationSummary>,
}

/// Summary of a consolidation dry-run triggered by session_end.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationSummary {
    pub promotions: usize,
    pub merges: usize,
}

/// Summary of a file_save hook — either a code reindex (source files)
/// or a file sync + embedding (.cogz/ entity files).
#[derive(Debug, Clone, Serialize)]
pub struct ReindexSummary {
    /// True if a code reindex was triggered (source file saved).
    pub reindexed: bool,
    /// True if a .cogz/ file sync was triggered (entity file saved).
    pub synced: bool,
    pub created: usize,
    pub updated: usize,
    pub marked_stale: usize,
    pub stale_knowledge_flagged: usize,
    /// Entities embedded after sync (0 if model unavailable).
    pub embedded: usize,
}

/// Handle a lifecycle event: record it, optionally assemble a context
/// pack, optionally record an observation.
pub fn handle_lifecycle_event(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &Path,
    query_model: &OnnxEmbeddingModel,
    input: &LifecycleInput,
) -> Result<LifecycleOutput, LifecycleError> {
    let event_type = input.event.event_type();

    // Build the event payload.
    let payload = match input.event {
        LifecycleEvent::SessionStart => json!({}),
        LifecycleEvent::PromptSubmit => {
            json!({ "prompt": input.prompt.unwrap_or("") })
        }
        LifecycleEvent::PreToolUse => {
            json!({ "tool_name": input.tool_name.unwrap_or("") })
        }
        LifecycleEvent::PostToolUse => {
            json!({
                "tool_name": input.tool_name.unwrap_or(""),
                "tool_result": input.tool_result.unwrap_or(""),
            })
        }
        LifecycleEvent::FileSave => {
            json!({ "file_path": input.file_path.unwrap_or("") })
        }
        LifecycleEvent::SessionEnd => json!({}),
    };

    // Record the event.
    let event_id = {
        let conn = storage.conn();
        events::record_event(&conn, event_type, None, &payload)?
    };

    // Assemble context pack for session_start and prompt_submit.
    let context_pack = match input.event {
        LifecycleEvent::SessionStart => Some(assemble_pack(
            storage,
            config,
            query_model,
            ContextMode::ColdStart,
            None,
        )?),
        LifecycleEvent::PromptSubmit => {
            let query = input.prompt;
            Some(assemble_pack(
                storage,
                config,
                query_model,
                ContextMode::Task,
                query,
            )?)
        }
        _ => None,
    };

    // Record observation for post_tool_use when both tool_name and
    // tool_result are provided. This is a lightweight heuristic —
    // every post_tool_use with both fields produces an observation.
    // Refinement (filtering, summarization) can come in Phase 12
    // based on dogfooding data.
    let observation_id = if input.event == LifecycleEvent::PostToolUse {
        if let (Some(name), Some(result)) = (input.tool_name, input.tool_result) {
            Some(record_tool_observation(
                storage,
                cogz_dir,
                query_model,
                name,
                result,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    // For file_save, either reindex source code (source files) or
    // sync + embed .cogz/ entity files. This keeps both the code index
    // and the knowledge DB fresh without requiring a manual `cogz
    // reindex` after every change.
    let reindex_summary = if input.event == LifecycleEvent::FileSave {
        Some(crate::hooks::handlers::handle_file_save(
            storage,
            config,
            cogz_dir,
            query_model,
            input.file_path,
        ))
    } else {
        None
    };

    // For session_end, run consolidation (promotion + merge) for real.
    // The configured thresholds are the safety mechanism — if they're
    // met, the system acts. This aligns with the first principle that
    // consolidation is continuous, not batch. Rules created by promotion
    // are git-tracked and reviewable; merged entities are superseded
    // (not deleted) and remain in the graph.
    let consolidation_summary = if input.event == LifecycleEvent::SessionEnd {
        Some(crate::hooks::handlers::handle_session_end(
            storage, config, cogz_dir,
        ))
    } else {
        None
    };

    Ok(LifecycleOutput {
        event_id,
        context_pack,
        observation_id,
        reindex_summary,
        consolidation_summary,
    })
}

/// Assemble a context pack, embedding the query if a model is available.
fn assemble_pack(
    storage: &Arc<Storage>,
    config: &Config,
    query_model: &OnnxEmbeddingModel,
    mode: ContextMode,
    query: Option<&str>,
) -> Result<ContextPack, LifecycleError> {
    let knowledge_embedding = query.and_then(|q| {
        use crate::embed::EmbeddingModel;
        query_model
            .embed_query(&[q])
            .ok()
            .and_then(|v| v.into_iter().next())
    });

    let pack = {
        let conn = storage.conn();
        assemble_context(
            &conn,
            &AssembleParams {
                mode,
                query,
                knowledge_embedding: knowledge_embedding.as_deref(),
                code_embedding: None,
                max_tokens: None,
                include_stale: false,
            },
            config,
        )?
    };

    Ok(pack)
}

/// Record an observation about a tool use. Follows the file-first
/// invariant: writes the file, then syncs to DB, then embeds.
fn record_tool_observation(
    storage: &Arc<Storage>,
    cogz_dir: &Path,
    query_model: &OnnxEmbeddingModel,
    tool_name: &str,
    tool_result: &str,
) -> Result<String, LifecycleError> {
    use crate::embed::EmbeddingModel;
    use crate::files::embed_sync::store_embeddings;

    let title = format!("Tool use: {}", tool_name);
    let content = format!("Tool `{}` was used.\n\nResult:\n{}", tool_name, tool_result);

    let mut entity = EntityFile::new(&title, FileEntityType::Observation, &content);
    entity
        .frontmatter
        .insert("source", FmValue::String("hook".to_string()));
    entity.frontmatter.insert("confidence", FmValue::Float(0.3));

    let path = entity.file_path(cogz_dir);
    write_entity_file(&path, &entity)?;

    // Sync the new file to the DB. The file is already written (file-first
    // invariant holds), so sync failure is recoverable — the next reindex
    // will pick it up. Log a warning rather than failing the event.
    let sync_result = sync_incremental(storage, cogz_dir);
    if !sync_result.errors.is_empty() {
        tracing::warn!(
            "sync_incremental failed after recording tool observation: {} error(s), first: {}",
            sync_result.errors.len(),
            sync_result.errors[0].error
        );
    }

    // Embed the observation text so it's visible to vector search.
    // Mirrors the MCP write path: embed without holding the DB lock,
    // then store under the lock. Graceful degradation if model absent.
    if query_model.is_available() {
        let text = format!("{}\n\n{}", title, content);
        if let Ok(embeddings) = query_model.embed(&[&text])
            && let Some(emb) = embeddings.into_iter().next()
        {
            let mut conn = storage.conn();
            store_embeddings(
                &mut conn,
                &[(
                    entity.id.clone(),
                    entity.entity_type.as_str().to_string(),
                    emb,
                )],
            );
        }
    }

    Ok(entity.id)
}

/// Error during lifecycle event handling.
#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("context assembly error: {0}")]
    Context(#[from] crate::context::AssembleError),
    #[error("file write error: {0}")]
    FileWrite(#[from] std::io::Error),
}
