//! Hybrid search — FTS5 + dual-model vector search fused via RRF,
//! with optional graph expansion.
//!
//! Knowledge and code entities live in separate embedding spaces.
//! The search function runs KNN per space (when the corresponding
//! query embedding is available) and fuses all ranked lists via RRF.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::config::SearchConfig;
use crate::storage::crud::{Entity, get_entities_batch};
use crate::storage::embeddings::{EmbeddingSpace, knn_search};
use crate::storage::query::fts_search;

use super::QueryEmbeddings;
use super::SearchError;
use super::describe::build_path_descriptions_batch;
use super::expand::expand_with_paths;
use super::rrf::fuse;
use super::{SearchMode, SearchParams, SearchResult, SearchResults};

/// Run a hybrid search.
///
/// Runs FTS5 always. Runs knowledge KNN when `embeddings.knowledge` is
/// provided, code KNN when `embeddings.code` is provided. Fuses all
/// available ranked lists via RRF. When `params.expand` is true,
/// follows graph edges from the top results to find related entities.
pub fn search(
    conn: &Connection,
    query: &str,
    embeddings: QueryEmbeddings<'_>,
    params: &SearchParams,
    config: &SearchConfig,
) -> Result<SearchResults, SearchError> {
    let limit = params.limit as i64;
    let type_filter = params.entity_type.as_deref();
    let status_filter = resolve_status_filter(params.status.as_deref());

    // 1. FTS search — returns full entities, cached to avoid re-fetching
    let fts_entities = fts_search(conn, query, type_filter, status_filter, limit)?;
    let fts_ids: Vec<String> = fts_entities.iter().map(|e| e.id.clone()).collect();

    // Seed entity_map with FTS results so we don't re-fetch them later
    let mut entity_map: std::collections::HashMap<String, Entity> = fts_entities
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    // 2. Knowledge vector search
    let knowledge_ids = if let Some(k_emb) = embeddings.knowledge {
        knn_channel(
            conn,
            k_emb,
            EmbeddingSpace::Knowledge,
            &mut entity_map,
            type_filter,
            status_filter,
            limit,
        )?
    } else {
        Vec::new()
    };

    // 3. Code vector search
    let code_ids = if let Some(c_emb) = embeddings.code {
        knn_channel(
            conn,
            c_emb,
            EmbeddingSpace::Code,
            &mut entity_map,
            type_filter,
            status_filter,
            limit,
        )?
    } else {
        Vec::new()
    };

    // 4. Determine search mode and run RRF fusion
    let search_mode = match (embeddings.knowledge.is_some(), embeddings.code.is_some()) {
        (true, true) => SearchMode::Hybrid,
        (true, false) => SearchMode::KnowledgeHybrid,
        (false, true) => SearchMode::CodeHybrid,
        (false, false) => SearchMode::FtsOnly,
    };

    let fused = if search_mode == SearchMode::FtsOnly {
        fts_ids
            .iter()
            .enumerate()
            .map(|(rank, id)| {
                let score = config.fts_weight / (config.rrf_k as f64 + (rank + 1) as f64);
                (id.clone(), score)
            })
            .collect::<Vec<_>>()
    } else {
        let mut lists: Vec<(&[String], f64)> = vec![(&fts_ids, config.fts_weight)];
        if !knowledge_ids.is_empty() {
            lists.push((&knowledge_ids, config.vec_weight));
        }
        if !code_ids.is_empty() {
            lists.push((&code_ids, config.code_vec_weight));
        }
        fuse(&lists, config.rrf_k)
    };

    // 5. Select top results
    let top_n: Vec<(String, f64)> = fused.into_iter().take(params.limit as usize).collect();

    // 6. Build direct search results (entities already in entity_map)
    let mut results: Vec<SearchResult> = top_n
        .iter()
        .filter_map(|(id, score)| {
            entity_map.get(id).map(|entity| SearchResult {
                entity: entity.clone(),
                relevance: *score as f32,
                graph_path: vec![id.clone()],
                graph_path_description: String::new(),
            })
        })
        .collect();

    // 7. Graph expansion
    if params.expand && params.max_hops > 0 && !results.is_empty() {
        let seed_ids: Vec<String> = results.iter().map(|r| r.entity.id.clone()).collect();
        let exclude_ids: HashSet<String> = results.iter().map(|r| r.entity.id.clone()).collect();

        // Build seed relevance map for decayed scoring of expanded entities
        let seed_relevance: HashMap<String, f32> = results
            .iter()
            .map(|r| (r.entity.id.clone(), r.relevance))
            .collect();

        let expansions = expand_with_paths(
            conn,
            &seed_ids,
            params.max_hops,
            &exclude_ids,
            status_filter,
        )?;

        let uncached_expansion_ids: Vec<String> = expansions
            .iter()
            .map(|e| e.entity_id.clone())
            .filter(|id| !entity_map.contains_key(id))
            .collect();
        let fetched = get_entities_batch(conn, &uncached_expansion_ids)?;
        for entity in fetched {
            entity_map.insert(entity.id.clone(), entity);
        }

        let paths_to_describe: Vec<Vec<String>> =
            expansions.iter().map(|e| e.graph_path.clone()).collect();
        let descriptions =
            build_path_descriptions_batch(conn, &paths_to_describe).unwrap_or_default();

        let mut expanded_results: Vec<SearchResult> = Vec::new();
        let mut seen_expanded: HashSet<String> = HashSet::new();
        for (exp, desc) in expansions.into_iter().zip(descriptions) {
            if !seen_expanded.insert(exp.entity_id.clone()) {
                continue;
            }
            if let Some(entity) = entity_map.get(&exp.entity_id) {
                // Relevance decays with hop distance: 0.5^hops
                let hop = exp.graph_path.len().saturating_sub(1);
                let seed_score = seed_relevance.get(&exp.seed_id).copied().unwrap_or(0.0);
                let decayed = seed_score * 0.5_f32.powi(hop as i32);
                expanded_results.push(SearchResult {
                    entity: entity.clone(),
                    relevance: decayed,
                    graph_path: exp.graph_path,
                    graph_path_description: desc,
                });
            }
        }
        results.extend(expanded_results);
    }

    // Track access counts for returned entities (derived state for
    // composite scoring). Only direct results, not graph expansions.
    let accessed_ids: Vec<String> = results.iter().map(|r| r.entity.id.clone()).collect();
    if !accessed_ids.is_empty() {
        let _ = crate::storage::access::increment_access_batch(conn, &accessed_ids);
    }

    Ok(SearchResults {
        results,
        search_mode,
    })
}

/// Run a single KNN channel: KNN search, batch-fetch entities, filter
/// by type/status, return filtered ID list.
fn knn_channel(
    conn: &Connection,
    query: &[f32],
    space: EmbeddingSpace,
    entity_map: &mut std::collections::HashMap<String, Entity>,
    type_filter: Option<&str>,
    status_filter: Option<&str>,
    limit: i64,
) -> Result<Vec<String>, SearchError> {
    let knn_limit = limit * 3;
    let knn_results = knn_search(conn, space, query, knn_limit)?;

    let uncached_ids: Vec<String> = knn_results
        .iter()
        .map(|(id, _)| id.clone())
        .filter(|id| !entity_map.contains_key(id))
        .collect();
    let fetched = get_entities_batch(conn, &uncached_ids)?;
    for entity in fetched {
        entity_map.insert(entity.id.clone(), entity);
    }

    let mut filtered = Vec::new();
    for (id, _) in knn_results {
        let Some(entity) = entity_map.get(&id) else {
            continue;
        };
        if type_filter.is_none_or(|t| entity.r#type == t)
            && status_filter.is_none_or(|s| entity.status == s)
        {
            filtered.push(id);
            if filtered.len() >= limit as usize {
                break;
            }
        }
    }
    Ok(filtered)
}

/// Resolve the status filter: None and "active" → Some("active"),
/// "all" → None (no filter), anything else → Some(value).
fn resolve_status_filter(status: Option<&str>) -> Option<&str> {
    match status {
        None => Some("active"),
        Some("all") => None,
        Some(s) => Some(s),
    }
}

#[cfg(test)]
#[path = "hybrid_tests.rs"]
mod tests;
