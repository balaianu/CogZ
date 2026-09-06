//! Tests for hybrid search.

use super::super::super::storage::crud::insert_entity;
use super::super::super::storage::edges::Edge;
use super::super::super::storage::embeddings::insert_embedding;
use super::super::super::storage::ensure_vec_extension;
use super::super::super::storage::schema::run_migrations;
use super::*;

#[path = "hybrid_tests_extra.rs"]
mod hybrid_tests_extra;

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
