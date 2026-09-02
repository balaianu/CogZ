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
        &default_config(),
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
        &default_config(),
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
        &default_config(),
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
        &default_config(),
    )
    .unwrap();
    assert_eq!(results.results.len(), 1);
}
