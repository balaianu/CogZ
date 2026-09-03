//! Hybrid search — FTS5 + dual-model vector search fused via RRF,
//! with optional graph expansion.
//!
//! Knowledge and code entities live in separate embedding spaces.
//! The search function runs KNN per space (when the corresponding
//! query embedding is available) and fuses all ranked lists via RRF.
//!
//! Source-type balancing: FTS results are split by entity type (code
//! vs knowledge) and fused separately. KNN similarity scores from
//! each embedding space determine the merge proportion — a query that
//! is semantically closer to code entities gets a higher code weight.
//! This prevents text-dense knowledge entries from dominating FTS
//! ranking when the query is about code.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::config::SearchConfig;
use crate::storage::crud::{Entity, EntityType, get_entities_batch};
use crate::storage::embeddings::{EmbeddingSpace, knn_search};
use crate::storage::query::{count_active_code, count_active_knowledge, fts_search};

use super::QueryEmbeddings;
use super::SearchError;
use super::balance::detect_proportions;
use super::describe::build_path_descriptions_batch;
use super::expand::expand_with_paths;
use super::rrf::fuse;
use super::{SearchMode, SearchParams, SearchResult, SearchResults};

/// Run a hybrid search.
///
/// Runs FTS5 always. Runs knowledge KNN when `embeddings.knowledge` is
/// provided, code KNN when `embeddings.code` is provided. FTS results
/// are split by entity type (code vs knowledge) and fused separately
/// with the corresponding KNN results. KNN similarity scores determine
/// the merge proportion between code and knowledge. When `params.expand`
/// is true, follows graph edges from the top results to find related
/// entities.
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
    let fts_entities = fts_search(
        conn,
        query,
        type_filter,
        status_filter,
        !params.include_tests,
        limit,
    )?;

    // Seed entity_map with FTS results so we don't re-fetch them later
    let mut entity_map: std::collections::HashMap<String, Entity> = fts_entities
        .iter()
        .map(|e| (e.id.clone(), e.clone()))
        .collect();

    // 2. Split FTS results by entity type (code vs knowledge)
    let (code_fts_ids, knowledge_fts_ids) = split_fts_by_type(&fts_entities);

    // 3. Knowledge vector search (returns IDs + distances for proportion detection)
    let (knowledge_ids, knowledge_distances) = if let Some(k_emb) = embeddings.knowledge {
        knn_channel(
            conn,
            k_emb,
            EmbeddingSpace::Knowledge,
            &mut entity_map,
            type_filter,
            status_filter,
            params.include_tests,
            limit,
        )?
    } else {
        (Vec::new(), Vec::new())
    };

    // 4. Code vector search
    let (code_ids, code_distances) = if let Some(c_emb) = embeddings.code {
        knn_channel(
            conn,
            c_emb,
            EmbeddingSpace::Code,
            &mut entity_map,
            type_filter,
            status_filter,
            params.include_tests,
            limit,
        )?
    } else {
        (Vec::new(), Vec::new())
    };

    // 5. Determine search mode
    let search_mode = match (embeddings.knowledge.is_some(), embeddings.code.is_some()) {
        (true, true) => SearchMode::Hybrid,
        (true, false) => SearchMode::KnowledgeHybrid,
        (false, true) => SearchMode::CodeHybrid,
        (false, false) => SearchMode::FtsOnly,
    };

    // 6. Detect source-type proportions from KNN similarity scores.
    //    When models are available, the query's semantic closeness to
    //    each embedding space determines how much weight code vs
    //    knowledge gets in the fused ranking. FTS-only mode uses 50/50.
    let (code_prop, knowledge_prop) =
        if search_mode == SearchMode::FtsOnly || !config.source_balance_enabled {
            (0.5, 0.5)
        } else {
            // Collection sizes for normalizing FTS match rates. Code entities
            // vastly outnumber knowledge entities, so raw match counts bias
            // toward code. Match rate (matches / collection_size) corrects this.
            let code_size = count_active_code(conn).unwrap_or(0) as usize;
            let knowledge_size = count_active_knowledge(conn).unwrap_or(0) as usize;
            detect_proportions(
                &code_distances,
                &knowledge_distances,
                code_fts_ids.len(),
                knowledge_fts_ids.len(),
                code_size,
                knowledge_size,
                config.min_source_proportion,
            )
        };

    tracing::debug!(
        code_proportion = code_prop,
        knowledge_proportion = knowledge_prop,
        search_mode = search_mode.as_str(),
        "source balance detected"
    );

    // 7. Fuse per source type with original RRF weights (no proportion
    //    scaling). Proportions are applied after normalization, not to
    //    the RRF weights, so that uneven weights (e.g. vec_weight=0.6
    //    vs code_vec_weight=0.3) don't counteract the balance.
    let code_fused = if !code_fts_ids.is_empty() || !code_ids.is_empty() {
        let mut lists: Vec<(&[String], f64)> = Vec::new();
        if !code_fts_ids.is_empty() {
            lists.push((&code_fts_ids, config.fts_weight));
        }
        if !code_ids.is_empty() {
            lists.push((&code_ids, config.code_vec_weight));
        }
        fuse(&lists, config.rrf_k)
    } else {
        Vec::new()
    };

    let knowledge_fused = if !knowledge_fts_ids.is_empty() || !knowledge_ids.is_empty() {
        let mut lists: Vec<(&[String], f64)> = Vec::new();
        if !knowledge_fts_ids.is_empty() {
            lists.push((&knowledge_fts_ids, config.fts_weight));
        }
        if !knowledge_ids.is_empty() {
            lists.push((&knowledge_ids, config.vec_weight));
        }
        fuse(&lists, config.rrf_k)
    } else {
        Vec::new()
    };

    // 8. Normalize each fused list to [0, 1] and apply proportions.
    //    Normalization makes scores comparable across source types
    //    regardless of RRF weight differences. Proportions then
    //    directly control the relative ranking between code and
    //    knowledge.
    let code_normalized = normalize_scores(&code_fused);
    let knowledge_normalized = normalize_scores(&knowledge_fused);

    let mut fused: Vec<(String, f64)> = code_normalized
        .into_iter()
        .map(|(id, score)| (id, score * code_prop))
        .chain(
            knowledge_normalized
                .into_iter()
                .map(|(id, score)| (id, score * knowledge_prop)),
        )
        .collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // 9. Select top results
    let top_n: Vec<(String, f64)> = fused.into_iter().take(params.limit as usize).collect();

    // 10. Build direct search results (entities already in entity_map)
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

    // Track access counts for direct results only (derived state for
    // composite scoring). Graph expansions are context, not retrieval.
    let accessed_ids: Vec<String> = results.iter().map(|r| r.entity.id.clone()).collect();
    if !accessed_ids.is_empty() {
        let _ = crate::storage::access::increment_access_batch(conn, &accessed_ids);
    }

    // Extend with expanded results after access tracking.
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
            params.include_tests,
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
                // Relevance decays aggressively with hop distance: 0.3^hops.
                // This creates clear separation between direct matches and
                // graph-expanded entities, so the token budget prioritizes
                // direct hits over tangential graph connections.
                // 1-hop: 30%, 2-hop: 9%, 3-hop: 2.7%
                let hop = exp.graph_path.len().saturating_sub(1);
                let seed_score = seed_relevance.get(&exp.seed_id).copied().unwrap_or(0.0);
                let decayed = seed_score * 0.3_f32.powi(hop as i32);
                expanded_results.push(SearchResult {
                    entity: entity.clone(),
                    relevance: decayed,
                    graph_path: exp.graph_path,
                    graph_path_description: desc,
                });
            }
        }

        // Cap expanded results to prevent graph fan-out from flooding
        // the result set. Expanded entities are context, not primary
        // matches — a small number suffices.
        let max_expansions = params.limit as usize;
        if expanded_results.len() > max_expansions {
            expanded_results.sort_by(|a, b| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            expanded_results.truncate(max_expansions);
        }

        results.extend(expanded_results);
    }

    Ok(SearchResults {
        results,
        search_mode,
    })
}

/// Run a single KNN channel: KNN search, batch-fetch entities, filter
/// by type/status/test-path. Returns filtered ID list and the
/// distances of the filtered results (for source-type proportion
/// detection).
#[allow(clippy::too_many_arguments)]
fn knn_channel(
    conn: &Connection,
    query: &[f32],
    space: EmbeddingSpace,
    entity_map: &mut std::collections::HashMap<String, Entity>,
    type_filter: Option<&str>,
    status_filter: Option<&str>,
    include_tests: bool,
    limit: i64,
) -> Result<(Vec<String>, Vec<f32>), SearchError> {
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

    let mut filtered_ids = Vec::new();
    let mut filtered_distances = Vec::new();
    for (id, dist) in knn_results {
        let Some(entity) = entity_map.get(&id) else {
            continue;
        };
        if type_filter.is_none_or(|t| entity.r#type == t)
            && status_filter.is_none_or(|s| entity.status == s)
            && (include_tests || !is_test_entity(entity))
        {
            filtered_ids.push(id);
            filtered_distances.push(dist);
            if filtered_ids.len() >= limit as usize {
                break;
            }
        }
    }
    Ok((filtered_ids, filtered_distances))
}

/// Normalize RRF scores to [0, 1] range using max normalization.
/// The highest score becomes 1.0, others scale proportionally.
/// An empty list returns empty.
fn normalize_scores(scores: &[(String, f64)]) -> Vec<(String, f64)> {
    if scores.is_empty() {
        return Vec::new();
    }
    let max_score = scores.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);
    if max_score <= 0.0 {
        return scores.to_vec();
    }
    scores
        .iter()
        .map(|(id, s)| (id.clone(), s / max_score))
        .collect()
}

/// Split FTS results into code and knowledge entity ID lists.
/// Code entities: function, class, file, module.
/// Knowledge entities: observation, rule, knowledge.
fn split_fts_by_type(fts_entities: &[Entity]) -> (Vec<String>, Vec<String>) {
    let mut code = Vec::new();
    let mut knowledge = Vec::new();
    for entity in fts_entities {
        let etype = match EntityType::parse(&entity.r#type) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if etype.is_code() {
            code.push(entity.id.clone());
        } else {
            knowledge.push(entity.id.clone());
        }
    }
    (code, knowledge)
}

/// Check if an entity is test code based on its file_path.
fn is_test_entity(entity: &Entity) -> bool {
    let Some(ref fp) = entity.file_path else {
        return false;
    };
    fp.starts_with("tests/")
        || fp.contains("/tests/")
        || fp.ends_with("/tests.rs")
        || fp.ends_with("_tests.rs")
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
