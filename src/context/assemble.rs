//! Context pack assembly — builds a [`ContextPack`] from search
//! results or recent entities, depending on the mode.

use rusqlite::Connection;

use crate::config::{Config, SearchConfig};
use crate::search::{self, QueryEmbeddings, SearchMode, SearchParams, SearchResult};
use crate::storage::crud::Entity;
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
    /// Optional knowledge-model query embedding for hybrid search.
    pub knowledge_embedding: Option<&'a [f32]>,
    /// Optional code-model query embedding for code-aware search.
    pub code_embedding: Option<&'a [f32]>,
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
            knowledge_embedding: None,
            code_embedding: None,
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
                params.knowledge_embedding,
                params.code_embedding,
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

/// Build sections for cold_start mode: repo identity, code map,
/// scored rules, and knowledge index.
///
/// Uses composite scoring (confidence + access frequency + recency
/// decay) instead of pure recency to select the most relevant rules
/// and knowledge. Includes a bounded code map summary so the agent
/// knows the project structure without flooding the token budget.
fn cold_start_sections(
    conn: &Connection,
    config: &Config,
    status: Option<&str>,
) -> Result<Vec<ContextSection>, AssembleError> {
    let now = chrono::Utc::now();
    let weights = crate::search::scoring::ScoreWeights::default();

    let mut sections = Vec::new();

    // 1. Repo identity
    sections.push(ContextSection {
        source: "identity".to_string(),
        entity_id: "repo".to_string(),
        title: config.project.name.clone(),
        content: format!("Project: {}", config.project.name),
        relevance: 0.0,
        graph_path: vec![],
    });

    // 2. Code map summary — top modules and files by connectivity.
    sections.extend(code_map_sections(conn, config));

    // 3. Scored rules — selected by composite score, not just recency.
    let rules = get_entities_by_type(conn, "rule", status, 1000)?;
    let rule_ids: Vec<String> = rules.iter().map(|e| e.id.clone()).collect();
    let access_counts =
        crate::storage::access::get_access_counts_batch(conn, &rule_ids).unwrap_or_default();

    let mut scored_rules: Vec<(f64, &Entity)> = rules
        .iter()
        .map(|e| {
            let count = access_counts.get(&e.id).copied().unwrap_or(0);
            let score = crate::search::scoring::cold_start_score(e, count, &now, &weights);
            (score, e)
        })
        .collect();
    scored_rules.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    for (_, entity) in scored_rules.iter().take(config.context.cold_start_rules) {
        let id = entity.id.clone();
        sections.push(ContextSection {
            source: "rule".to_string(),
            entity_id: id.clone(),
            title: entity.title.clone().unwrap_or_default(),
            content: entity.content.clone(),
            relevance: 0.0,
            graph_path: vec![id],
        });
    }

    // 4. Knowledge index — titles only for most entries, full content
    // for the top scored entry.
    let knowledge = get_entities_by_type(conn, "knowledge", status, 1000)?;
    let knowledge_ids: Vec<String> = knowledge.iter().map(|e| e.id.clone()).collect();
    let knowledge_access =
        crate::storage::access::get_access_counts_batch(conn, &knowledge_ids).unwrap_or_default();

    let mut scored_knowledge: Vec<(f64, &Entity)> = knowledge
        .iter()
        .map(|e| {
            let count = knowledge_access.get(&e.id).copied().unwrap_or(0);
            let score = crate::search::scoring::cold_start_score(e, count, &now, &weights);
            (score, e)
        })
        .collect();
    scored_knowledge.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Full content for top entry, titles only for the rest.
    if let Some((_, top)) = scored_knowledge.first() {
        let id = top.id.clone();
        sections.push(ContextSection {
            source: "knowledge".to_string(),
            entity_id: id.clone(),
            title: top.title.clone().unwrap_or_default(),
            content: top.content.clone(),
            relevance: 0.0,
            graph_path: vec![id],
        });
    }

    // Title-only index for remaining knowledge entries.
    let remaining: Vec<String> = scored_knowledge
        .iter()
        .skip(1)
        .map(|(_, e)| format!("- {}", e.title.as_deref().unwrap_or("(untitled)")))
        .collect();
    if !remaining.is_empty() {
        sections.push(ContextSection {
            source: "knowledge_index".to_string(),
            entity_id: "index".to_string(),
            title: "Knowledge Index".to_string(),
            content: remaining.join("\n"),
            relevance: 0.0,
            graph_path: vec![],
        });
    }

    Ok(sections)
}

/// Build a bounded code map summary: top modules and key files.
fn code_map_sections(conn: &Connection, _config: &Config) -> Vec<ContextSection> {
    let mut sections = Vec::new();

    // Top modules by entity count.
    let modules = get_entities_by_type(conn, "module", Some("active"), 20).unwrap_or_default();
    if !modules.is_empty() {
        let module_list: Vec<String> = modules
            .iter()
            .map(|m| {
                format!(
                    "- {} ({})",
                    m.title.as_deref().unwrap_or("(unnamed)"),
                    m.file_path.as_deref().unwrap_or("?")
                )
            })
            .collect();
        sections.push(ContextSection {
            source: "code_map".to_string(),
            entity_id: "modules".to_string(),
            title: "Code Map — Modules".to_string(),
            content: module_list.join("\n"),
            relevance: 0.0,
            graph_path: vec![],
        });
    }

    // Key files — those with the most contains edges (high connectivity).
    let key_files: Vec<(String, String, i64)> = {
        let mut stmt = match conn.prepare(
            "SELECT e.title, e.file_path, COUNT(*) as edge_count
             FROM edges ed
             JOIN entities e ON ed.source_id = e.id
             WHERE ed.edge_type = 'contains' AND e.type = 'file'
             GROUP BY e.id
             ORDER BY edge_count DESC
             LIMIT 15",
        ) {
            Ok(s) => s,
            Err(_) => return sections,
        };
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .ok();
        match rows {
            Some(rows) => rows.filter_map(|r| r.ok()).collect(),
            None => return sections,
        }
    };

    if !key_files.is_empty() {
        let file_list: Vec<String> = key_files
            .iter()
            .map(|(title, path, count)| format!("- {title} ({path}) — {count} symbols"))
            .collect();
        sections.push(ContextSection {
            source: "code_map".to_string(),
            entity_id: "key_files".to_string(),
            title: "Code Map — Key Files".to_string(),
            content: file_list.join("\n"),
            relevance: 0.0,
            graph_path: vec![],
        });
    }

    sections
}

/// Build sections from search results (task and escalation modes).
#[allow(clippy::too_many_arguments)]
fn query_sections(
    conn: &Connection,
    query: &str,
    knowledge_embedding: Option<&[f32]>,
    code_embedding: Option<&[f32]>,
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

    let embeddings = QueryEmbeddings {
        knowledge: knowledge_embedding,
        code: code_embedding,
    };
    let results = search::search(conn, query, embeddings, &params, search_config)?;
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
