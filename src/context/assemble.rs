//! Context pack assembly — builds a [`ContextPack`] from search
//! results or recent entities, depending on the mode.

use rusqlite::Connection;

use crate::config::{Config, SearchConfig};
use crate::search::{self, SearchMode, SearchParams, SearchResult};
use crate::storage::query::get_entities_by_type;

use super::compress::{fit_budget, section_tokens, sort_by_priority};
use super::modes::ContextMode;
use super::{ContextPack, ContextSection, PackMetadata};

/// Error during context assembly.
#[derive(Debug, thiserror::Error)]
pub enum AssembleError {
    #[error("search error: {0}")]
    Search(#[from] search::SearchError),
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("query required for {0} mode")]
    QueryRequired(ContextMode),
}

/// Parameters for assembling a context pack.
pub struct AssembleParams<'a> {
    /// The mode determines retrieval strategy and priorities.
    pub mode: ContextMode,
    /// The query for task/escalation modes. Ignored for cold_start.
    pub query: Option<&'a str>,
    /// Optional query embedding for hybrid search. If None, FTS-only.
    pub query_embedding: Option<&'a [f32]>,
    /// Override the token budget from config. If None, use config default.
    pub max_tokens: Option<usize>,
    /// Include stale entities in results.
    pub include_stale: bool,
}

impl Default for AssembleParams<'_> {
    fn default() -> Self {
        Self {
            mode: ContextMode::Task,
            query: None,
            query_embedding: None,
            max_tokens: None,
            include_stale: false,
        }
    }
}

/// Assemble a context pack.
pub fn assemble_context(
    conn: &Connection,
    params: &AssembleParams<'_>,
    config: &Config,
) -> Result<ContextPack, AssembleError> {
    let token_budget = params
        .max_tokens
        .unwrap_or(config.context.default_token_budget);
    // For cold_start: None means no filter (all statuses), Some("active") filters to active.
    // For search: the resolve_status_filter function maps None → active, "all" → no filter.
    let cold_start_status = if params.include_stale {
        None
    } else {
        Some("active")
    };
    let search_status = if params.include_stale {
        Some("all")
    } else {
        None
    };

    let (sections, search_mode) = match params.mode {
        ContextMode::ColdStart => {
            let sections = cold_start_sections(conn, config, cold_start_status)?;
            (sections, SearchMode::FtsOnly)
        }
        ContextMode::Task | ContextMode::Escalation => {
            let query = params
                .query
                .ok_or(AssembleError::QueryRequired(params.mode))?;
            let (max_results, max_hops) = if params.mode == ContextMode::Escalation {
                (
                    config.context.escalation_max_results,
                    config.context.escalation_max_hops,
                )
            } else {
                (
                    config.context.task_max_results,
                    config.context.task_max_hops,
                )
            };
            query_sections(
                conn,
                query,
                params.query_embedding,
                max_results,
                max_hops,
                search_status,
                &config.search,
            )?
        }
    };

    let mut sections = sections;
    sort_by_priority(&mut sections);
    let (kept, dropped) = fit_budget(sections, token_budget);

    let size_tokens = kept.iter().map(section_tokens).sum();
    let selected_sources: Vec<String> = {
        let mut seen: Vec<String> = Vec::new();
        for s in &kept {
            if !seen.contains(&s.source) {
                seen.push(s.source.clone());
            }
        }
        seen
    };

    Ok(ContextPack {
        query: params.query.unwrap_or("").to_string(),
        mode: params.mode,
        sections: kept,
        metadata: PackMetadata {
            size_tokens,
            selected_sources,
            dropped_sources: dropped,
            search_mode: search_mode.as_str().to_string(),
        },
    })
}

/// Build sections for cold_start mode: repo identity, recent rules,
/// and recent observations.
fn cold_start_sections(
    conn: &Connection,
    config: &Config,
    status: Option<&str>,
) -> Result<Vec<ContextSection>, AssembleError> {
    let rules = get_entities_by_type(conn, "rule", status, config.context.cold_start_rules as i64)?;
    let observations = get_entities_by_type(
        conn,
        "observation",
        status,
        config.context.cold_start_observations as i64,
    )?;

    let mut sections = Vec::new();

    // Repo identity as the first section — gives the agent context
    // about which project it's working in.
    sections.push(ContextSection {
        source: "identity".to_string(),
        entity_id: "repo".to_string(),
        title: config.project.name.clone(),
        content: format!("Project: {}", config.project.name),
        relevance: 0.0,
        graph_path: vec![],
    });

    for entity in rules {
        let id = entity.id;
        sections.push(ContextSection {
            source: "rule".to_string(),
            entity_id: id.clone(),
            title: entity.title.unwrap_or_default(),
            content: entity.content,
            relevance: 0.0,
            graph_path: vec![id],
        });
    }
    for entity in observations {
        let id = entity.id;
        sections.push(ContextSection {
            source: "observation".to_string(),
            entity_id: id.clone(),
            title: entity.title.unwrap_or_default(),
            content: entity.content,
            relevance: 0.0,
            graph_path: vec![id],
        });
    }
    Ok(sections)
}

/// Build sections from search results (task and escalation modes).
fn query_sections(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    max_results: u32,
    max_hops: usize,
    status: Option<&str>,
    search_config: &SearchConfig,
) -> Result<(Vec<ContextSection>, SearchMode), AssembleError> {
    let params = SearchParams {
        entity_type: None,
        status: status.map(|s| s.to_string()),
        limit: max_results,
        expand: max_hops > 0,
        max_hops,
    };

    let results = search::search(conn, query, query_embedding, &params, search_config)?;
    let search_mode = results.search_mode;

    let sections = results
        .results
        .into_iter()
        .map(search_result_to_section)
        .collect();

    Ok((sections, search_mode))
}

/// Convert a search result to a context section.
/// Takes ownership to avoid cloning — the caller's results are consumed.
fn search_result_to_section(result: SearchResult) -> ContextSection {
    ContextSection {
        source: result.entity.r#type,
        entity_id: result.entity.id,
        title: result.entity.title.unwrap_or_default(),
        content: result.entity.content,
        relevance: result.relevance,
        graph_path: result.graph_path,
    }
}
