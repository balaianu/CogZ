use super::*;
use crate::storage::crud::insert_entity;
use crate::storage::edges::insert_edge;
use crate::storage::embeddings::insert_embedding;

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
