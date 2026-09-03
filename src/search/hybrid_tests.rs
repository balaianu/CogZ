//! Tests for hybrid search.

use super::super::super::storage::crud::insert_entity;
use super::super::super::storage::edges::{Edge, insert_edge};
use super::super::super::storage::embeddings::insert_embedding;
use super::super::super::storage::ensure_vec_extension;
use super::super::super::storage::schema::run_migrations;
use super::*;

fn setup() -> Connection {
    ensure_vec_extension();
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, 768).unwrap();
    conn
}

fn default_config() -> SearchConfig {
    SearchConfig {
        fts_weight: 0.3,
        vec_weight: 0.4,
        code_vec_weight: 0.3,
        rrf_k: 60,
        max_results: 20,
        min_source_proportion: 0.2,
        source_balance_enabled: false,
    }
}

fn balanced_config() -> SearchConfig {
    SearchConfig {
        source_balance_enabled: true,
        ..default_config()
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
    let results = search(
        &conn,
        "ranking",
        QueryEmbeddings::none(),
        &params,
        &default_config(),
    )
    .unwrap();

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
    insert_embedding(&conn, "u1", "observation", &vec![0.1_f32; 768]).unwrap();

    let e2 = Entity::new("u2", "observation", "Other", "different content");
    insert_entity(&conn, &e2).unwrap();
    insert_embedding(&conn, "u2", "observation", &vec![0.9_f32; 768]).unwrap();

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let query_vec = vec![0.1_f32; 768];
    let results = search(
        &conn,
        "ranking",
        QueryEmbeddings::knowledge(&query_vec),
        &params,
        &default_config(),
    )
    .unwrap();

    assert_eq!(results.search_mode, SearchMode::KnowledgeHybrid);
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
    let results = search(
        &conn,
        "ranking",
        QueryEmbeddings::none(),
        &params,
        &default_config(),
    )
    .unwrap();

    assert_eq!(results.results.len(), 1);
    assert_eq!(results.results[0].entity.r#type, "observation");
}

#[test]
fn balanced_fusion_code_favored_when_query_closer_to_code() {
    let conn = setup();

    // Knowledge entity — matches FTS for "indexing"
    insert_entity(
        &conn,
        &Entity::new(
            "k1",
            "knowledge",
            "Search pipeline indexing",
            "indexing pipeline content",
        ),
    )
    .unwrap();
    // Code entity — also matches FTS for "indexing"
    insert_entity(
        &conn,
        &Entity::new(
            "f1",
            "function",
            "index_code",
            "fn index_code() { indexing }",
        ),
    )
    .unwrap();

    // Knowledge embedding: far from query
    insert_embedding(&conn, "k1", "knowledge", &vec![0.9_f32; 768]).unwrap();
    // Code embedding: close to query
    insert_embedding(&conn, "f1", "function", &vec![0.1_f32; 768]).unwrap();

    let query_vec = vec![0.1_f32; 768];
    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(
        &conn,
        "indexing",
        QueryEmbeddings::both(&query_vec, &query_vec),
        &params,
        &balanced_config(),
    )
    .unwrap();

    assert_eq!(results.search_mode, SearchMode::Hybrid);
    // Both should be present
    assert_eq!(results.results.len(), 2);
    // Code entity should rank higher — query embedding is closer to code space
    assert_eq!(results.results[0].entity.id, "f1");
    assert_eq!(results.results[1].entity.id, "k1");
}

#[test]
fn balanced_fusion_knowledge_favored_when_query_closer_to_knowledge() {
    let conn = setup();

    insert_entity(
        &conn,
        &Entity::new(
            "k1",
            "knowledge",
            "Search pipeline indexing",
            "indexing pipeline content",
        ),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new(
            "f1",
            "function",
            "index_code",
            "fn index_code() { indexing }",
        ),
    )
    .unwrap();

    // Knowledge embedding: close to query
    insert_embedding(&conn, "k1", "knowledge", &vec![0.1_f32; 768]).unwrap();
    // Code embedding: far from query
    insert_embedding(&conn, "f1", "function", &vec![0.9_f32; 768]).unwrap();

    let query_vec = vec![0.1_f32; 768];
    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(
        &conn,
        "indexing",
        QueryEmbeddings::both(&query_vec, &query_vec),
        &params,
        &balanced_config(),
    )
    .unwrap();

    assert_eq!(results.results.len(), 2);
    // Knowledge entity should rank higher — query embedding is closer to knowledge space
    assert_eq!(results.results[0].entity.id, "k1");
    assert_eq!(results.results[1].entity.id, "f1");
}

#[test]
fn balanced_fusion_fts_only_splits_by_type() {
    // In FTS-only mode, code and knowledge are fused separately with
    // equal proportions. Both should appear in results.
    let conn = setup();

    insert_entity(
        &conn,
        &Entity::new(
            "k1",
            "knowledge",
            "indexing design",
            "indexing architecture content",
        ),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new(
            "f1",
            "function",
            "index_code",
            "fn index_code() { indexing }",
        ),
    )
    .unwrap();

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(
        &conn,
        "indexing",
        QueryEmbeddings::none(),
        &params,
        &balanced_config(),
    )
    .unwrap();

    assert_eq!(results.search_mode, SearchMode::FtsOnly);
    assert_eq!(results.results.len(), 2);
    // Both code and knowledge should be present
    let types: Vec<&str> = results
        .results
        .iter()
        .map(|r| r.entity.r#type.as_str())
        .collect();
    assert!(types.contains(&"knowledge"));
    assert!(types.contains(&"function"));
}

#[test]
fn balanced_fusion_floor_prevents_knowledge_suppression() {
    // Even when code is very close and knowledge is very far,
    // the floor ensures knowledge entities still appear.
    let conn = setup();

    insert_entity(
        &conn,
        &Entity::new(
            "k1",
            "knowledge",
            "indexing design",
            "indexing architecture content",
        ),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new(
            "f1",
            "function",
            "index_code",
            "fn index_code() { indexing }",
        ),
    )
    .unwrap();

    // Code embedding: nearly identical to query
    insert_embedding(&conn, "f1", "function", &vec![0.01_f32; 768]).unwrap();
    // Knowledge embedding: nearly opposite to query
    insert_embedding(&conn, "k1", "knowledge", &vec![1.9_f32; 768]).unwrap();

    let query_vec = vec![0.01_f32; 768];
    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(
        &conn,
        "indexing",
        QueryEmbeddings::both(&query_vec, &query_vec),
        &params,
        &balanced_config(),
    )
    .unwrap();

    // Knowledge should still appear despite being far in vector space,
    // because the floor gives it at least 20% weight, and it matches FTS.
    let has_knowledge = results
        .results
        .iter()
        .any(|r| r.entity.r#type == "knowledge");
    assert!(
        has_knowledge,
        "knowledge entity should appear despite low vector similarity"
    );
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
    let results = search(
        &conn,
        "ranking",
        QueryEmbeddings::none(),
        &params,
        &balanced_config(),
    )
    .unwrap();

    // Direct result: obs1
    assert!(results.results.iter().any(|r| r.entity.id == "obs1"));
    // Expanded result: func1 (reached via references edge)
    let func1 = results.results.iter().find(|r| r.entity.id == "func1");
    assert!(func1.is_some());
    let func1 = func1.unwrap();
    assert_eq!(func1.graph_path, vec!["obs1", "func1"]);
    assert!(!func1.graph_path_description.is_empty());
    // Expanded results get decayed relevance from the seed
    assert!(
        func1.relevance > 0.0,
        "expanded result should have decayed relevance"
    );
}

#[test]
fn search_no_results() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "content")).unwrap();

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(
        &conn,
        "nonexistent",
        QueryEmbeddings::none(),
        &params,
        &balanced_config(),
    )
    .unwrap();
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
    let results = search(
        &conn,
        "ranking",
        QueryEmbeddings::none(),
        &params,
        &balanced_config(),
    )
    .unwrap();
    assert!(results.results.is_empty());

    // Status "all" — includes stale
    let params = SearchParams {
        status: Some("all".to_string()),
        expand: false,
        ..Default::default()
    };
    let results = search(
        &conn,
        "ranking",
        QueryEmbeddings::none(),
        &params,
        &balanced_config(),
    )
    .unwrap();
    assert_eq!(results.results.len(), 1);
}
