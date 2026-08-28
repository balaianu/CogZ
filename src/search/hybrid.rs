//! Hybrid search — FTS5 + vector search fused via RRF, with optional
//! graph expansion.
//!
//! The search function is model-agnostic: it takes an optional query
//! embedding and falls back to FTS-only when none is provided. Model
//! loading and query embedding happen at the CLI boundary.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::config::SearchConfig;
use crate::storage::crud::{Entity, get_entity};
use crate::storage::embeddings::knn_search;
use crate::storage::query::fts_search;

use super::SearchError;
use super::expand::{build_path_description, expand_with_paths};
use super::rrf::fuse;
use super::{SearchMode, SearchParams, SearchResult, SearchResults};

/// Run a hybrid search.
///
/// If `query_embedding` is provided, runs both FTS5 and vector search
/// and fuses results via RRF. If not, runs FTS5 only (graceful
/// degradation). When `params.expand` is true, follows graph edges
/// from the top results to find related entities, recording the path
/// to each.
pub fn search(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    params: &SearchParams,
    config: &SearchConfig,
) -> Result<SearchResults, SearchError> {
    let limit = params.limit as i64;
    let type_filter = params.entity_type.as_deref();
    let status_filter = resolve_status_filter(params.status.as_deref());

    // 1. FTS search
    let fts_entities = fts_search(conn, query, type_filter, status_filter, limit)?;
    let fts_ids: Vec<String> = fts_entities.iter().map(|e| e.id.clone()).collect();

    // 2. Vector search (if embedding available)
    let (vec_ids, search_mode) = if let Some(embedding) = query_embedding {
        // Over-fetch to compensate for post-KNN status/type filtering
        let knn_limit = limit * 3;
        let knn_results = knn_search(conn, embedding, knn_limit)?;
        let filtered: Vec<String> = knn_results
            .into_iter()
            .filter_map(|(id, _)| {
                get_entity(conn, &id)
                    .ok()
                    .filter(|e| type_filter.is_none_or(|t| e.r#type == t))
                    .filter(|e| status_filter.is_none_or(|s| e.status == s))
                    .map(|e| e.id)
            })
            .take(limit as usize)
            .collect();
        (filtered, SearchMode::Hybrid)
    } else {
        (Vec::new(), SearchMode::FtsOnly)
    };

    // 3. RRF fusion
    let fused = if search_mode == SearchMode::Hybrid {
        fuse(
            &[(&fts_ids, config.fts_weight), (&vec_ids, config.vec_weight)],
            config.rrf_k,
        )
    } else {
        // FTS-only: use FTS ranking directly, with fts_weight as the score
        fts_ids
            .iter()
            .enumerate()
            .map(|(rank, id)| {
                let score = config.fts_weight / (config.rrf_k as f64 + (rank + 1) as f64);
                (id.clone(), score)
            })
            .collect::<Vec<_>>()
    };

    // 4. Fetch entities for the top results
    let top_n: Vec<(String, f64)> = fused.into_iter().take(params.limit as usize).collect();

    let mut entity_map: std::collections::HashMap<String, Entity> =
        std::collections::HashMap::new();
    for (id, _) in &top_n {
        if let Ok(entity) = get_entity(conn, id) {
            entity_map.insert(id.clone(), entity);
        }
    }

    // 5. Build direct search results
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

    // 6. Graph expansion
    if params.expand && params.max_hops > 0 && !results.is_empty() {
        let seed_ids: Vec<String> = results.iter().map(|r| r.entity.id.clone()).collect();
        let exclude_ids: HashSet<String> = results.iter().map(|r| r.entity.id.clone()).collect();

        let expansions = expand_with_paths(
            conn,
            &seed_ids,
            params.max_hops,
            &exclude_ids,
            status_filter,
        )?;

        for exp in expansions {
            if let Ok(entity) = get_entity(conn, &exp.entity_id) {
                let description = build_path_description(conn, &exp.graph_path).unwrap_or_default();
                results.push(SearchResult {
                    entity,
                    relevance: 0.0,
                    graph_path: exp.graph_path,
                    graph_path_description: description,
                });
            }
        }
    }

    Ok(SearchResults {
        results,
        search_mode,
    })
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
mod tests {
    use super::super::super::storage::crud::insert_entity;
    use super::super::super::storage::edges::{Edge, insert_edge};
    use super::super::super::storage::embeddings::insert_embedding;
    use super::super::super::storage::ensure_vec_extension;
    use super::super::super::storage::schema::run_migrations;
    use super::*;

    fn setup() -> Connection {
        ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn default_config() -> SearchConfig {
        SearchConfig {
            fts_weight: 0.4,
            vec_weight: 0.6,
            rrf_k: 60,
            max_results: 20,
        }
    }

    fn edge(source: &str, target: &str, edge_type: &str) -> Edge {
        Edge {
            source_id: source.to_string(),
            target_id: target.to_string(),
            edge_type: edge_type.to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn fts_only_search() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new(
                "u1",
                "observation",
                "FTS5 ranking bug",
                "ranking issue content",
            ),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new("u2", "rule", "Unrelated", "completely different"),
        )
        .unwrap();

        let params = SearchParams {
            expand: false,
            ..Default::default()
        };
        let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

        assert_eq!(results.search_mode, SearchMode::FtsOnly);
        assert_eq!(results.results.len(), 1);
        assert_eq!(results.results[0].entity.id, "u1");
        assert!(results.results[0].relevance > 0.0);
    }

    #[test]
    fn hybrid_search() {
        let conn = setup();
        let e1 = Entity::new("u1", "observation", "FTS5 ranking", "ranking content");
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(&conn, "u1", &vec![0.1_f32; 768]).unwrap();

        let e2 = Entity::new("u2", "observation", "Other", "different content");
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(&conn, "u2", &vec![0.9_f32; 768]).unwrap();

        let params = SearchParams {
            expand: false,
            ..Default::default()
        };
        let query_vec = vec![0.1_f32; 768];
        let results = search(
            &conn,
            "ranking",
            Some(&query_vec),
            &params,
            &default_config(),
        )
        .unwrap();

        assert_eq!(results.search_mode, SearchMode::Hybrid);
        // u1 matches both FTS and vec, should be first
        assert_eq!(results.results[0].entity.id, "u1");
    }

    #[test]
    fn search_with_type_filter() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "ranking", "ranking content"),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new("u2", "rule", "ranking", "ranking content"),
        )
        .unwrap();

        let params = SearchParams {
            entity_type: Some("observation".to_string()),
            expand: false,
            ..Default::default()
        };
        let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

        assert_eq!(results.results.len(), 1);
        assert_eq!(results.results[0].entity.r#type, "observation");
    }

    #[test]
    fn search_with_graph_expansion() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("obs1", "observation", "FTS5 bug", "ranking bug content"),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new("func1", "function", "build_sql", "sql builder"),
        )
        .unwrap();

        insert_edge(&conn, &edge("obs1", "func1", "references")).unwrap();

        let params = SearchParams {
            expand: true,
            max_hops: 2,
            ..Default::default()
        };
        let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

        // Direct result: obs1
        assert!(results.results.iter().any(|r| r.entity.id == "obs1"));
        // Expanded result: func1 (reached via references edge)
        let func1 = results.results.iter().find(|r| r.entity.id == "func1");
        assert!(func1.is_some());
        let func1 = func1.unwrap();
        assert_eq!(func1.graph_path, vec!["obs1", "func1"]);
        assert!(!func1.graph_path_description.is_empty());
    }

    #[test]
    fn search_no_results() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "content")).unwrap();

        let params = SearchParams {
            expand: false,
            ..Default::default()
        };
        let results = search(&conn, "nonexistent", None, &params, &default_config()).unwrap();
        assert!(results.results.is_empty());
    }

    #[test]
    fn search_status_all_includes_stale() {
        let conn = setup();
        let mut e1 = Entity::new("u1", "observation", "ranking", "ranking content");
        e1.status = "stale".to_string();
        insert_entity(&conn, &e1).unwrap();

        // Default (active only) — no results
        let params = SearchParams {
            expand: false,
            ..Default::default()
        };
        let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();
        assert!(results.results.is_empty());

        // Status "all" — includes stale
        let params = SearchParams {
            status: Some("all".to_string()),
            expand: false,
            ..Default::default()
        };
        let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();
        assert_eq!(results.results.len(), 1);
    }
}
