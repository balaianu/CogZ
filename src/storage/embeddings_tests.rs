use super::super::crud::Entity;
use super::super::crud::insert_entity;
use super::super::ensure_vec_extension;
use super::super::schema::run_migrations;
use super::*;

fn setup() -> Connection {
    ensure_vec_extension();
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, 768).unwrap();
    conn
}

#[test]
fn insert_and_knn_search_knowledge() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "Test", "content")).unwrap();

    let embedding = vec![0.1_f32; 768];
    insert_embedding(&conn, "u1", "observation", &embedding).unwrap();

    let results = knn_search(&conn, EmbeddingSpace::Knowledge, &embedding, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "u1");
    assert!(results[0].1 < 0.001);
}

#[test]
fn insert_and_knn_search_code() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("f1", "function", "my_func", "fn my_func() {}"),
    )
    .unwrap();

    let embedding = vec![0.2_f32; 768];
    insert_embedding(&conn, "f1", "function", &embedding).unwrap();

    let results = knn_search(&conn, EmbeddingSpace::Code, &embedding, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "f1");
}

#[test]
fn knn_search_isolated_per_space() {
    let conn = setup();
    // Code entity with embedding close to query
    insert_entity(
        &conn,
        &Entity::new("f1", "function", "func", "fn func() {}"),
    )
    .unwrap();
    insert_embedding(&conn, "f1", "function", &vec![0.1_f32; 768]).unwrap();

    // Knowledge entity with embedding far from query
    insert_entity(&conn, &Entity::new("o1", "observation", "obs", "content")).unwrap();
    insert_embedding(&conn, "o1", "observation", &vec![0.9_f32; 768]).unwrap();

    let query = vec![0.1_f32; 768];

    // Code KNN should only find the function
    let code_results = knn_search(&conn, EmbeddingSpace::Code, &query, 10).unwrap();
    assert_eq!(code_results.len(), 1);
    assert_eq!(code_results[0].0, "f1");

    // Knowledge KNN should only find the observation
    let knowledge_results = knn_search(&conn, EmbeddingSpace::Knowledge, &query, 10).unwrap();
    assert_eq!(knowledge_results.len(), 1);
    assert_eq!(knowledge_results[0].0, "o1");
}

#[test]
fn knn_search_returns_nearest() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

    insert_embedding(&conn, "u1", "observation", &vec![0.1_f32; 768]).unwrap();
    insert_embedding(&conn, "u2", "observation", &vec![0.9_f32; 768]).unwrap();

    let query = vec![0.1_f32; 768];
    let results = knn_search(&conn, EmbeddingSpace::Knowledge, &query, 2).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "u1");
    assert!(results[0].1 < results[1].1);
}

#[test]
fn delete_entity_embedding() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "T", "c")).unwrap();
    insert_embedding(&conn, "u1", "observation", &vec![0.1_f32; 768]).unwrap();

    delete_embedding(&conn, "u1").unwrap();

    let results = knn_search(&conn, EmbeddingSpace::Knowledge, &vec![0.1_f32; 768], 1).unwrap();
    assert!(results.is_empty());
}

#[test]
fn knn_search_type_filter_excludes_other_types() {
    let conn = setup();
    // Insert an observation and a knowledge entry with identical
    // embeddings. Without the type filter, both would be returned.
    insert_entity(&conn, &Entity::new("obs1", "observation", "Obs", "c")).unwrap();
    insert_entity(&conn, &Entity::new("k1", "knowledge", "Knowledge", "c")).unwrap();
    insert_embedding(&conn, "obs1", "observation", &vec![0.1_f32; 768]).unwrap();
    insert_embedding(&conn, "k1", "knowledge", &vec![0.1_f32; 768]).unwrap();

    let query = vec![0.1_f32; 768];

    // Unfiltered: both entities are returned.
    let unfiltered = knn_search(&conn, EmbeddingSpace::Knowledge, &query, 10).unwrap();
    assert_eq!(unfiltered.len(), 2);

    // Filtered to observations only: knowledge entry is excluded.
    let filtered =
        knn_search_with_type_filter(&conn, EmbeddingSpace::Knowledge, &query, 10, "observation")
            .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0, "obs1");
}

#[test]
fn get_embedding_checks_both_tables() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("f1", "function", "func", "fn func() {}"),
    )
    .unwrap();
    insert_embedding(&conn, "f1", "function", &vec![0.5_f32; 768]).unwrap();

    let emb = get_embedding(&conn, "f1").unwrap();
    assert!(emb.is_some());
    assert_eq!(emb.unwrap().len(), 768);
}

#[test]
fn count_embeddings_across_both_tables() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("f1", "function", "func", "fn func() {}"),
    )
    .unwrap();
    insert_entity(&conn, &Entity::new("o1", "observation", "obs", "content")).unwrap();

    insert_embedding(&conn, "f1", "function", &vec![0.1_f32; 768]).unwrap();
    insert_embedding(&conn, "o1", "observation", &vec![0.2_f32; 768]).unwrap();

    assert_eq!(count_embeddings(&conn).unwrap(), 2);
}

#[test]
fn get_knowledge_embeddings_batch_returns_map() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("k1", "knowledge", "K1", "c")).unwrap();
    insert_entity(&conn, &Entity::new("k2", "knowledge", "K2", "c")).unwrap();
    insert_entity(&conn, &Entity::new("k3", "knowledge", "K3", "c")).unwrap();

    insert_embedding(&conn, "k1", "knowledge", &vec![0.1_f32; 768]).unwrap();
    insert_embedding(&conn, "k2", "knowledge", &vec![0.2_f32; 768]).unwrap();
    // k3 has no embedding

    let map = get_knowledge_embeddings_batch(
        &conn,
        &["k1".to_string(), "k2".to_string(), "k3".to_string()],
    )
    .unwrap();

    assert_eq!(map.len(), 2);
    assert!(map.contains_key("k1"));
    assert!(map.contains_key("k2"));
    assert!(!map.contains_key("k3"));
}

#[test]
fn get_knowledge_embeddings_batch_empty_input() {
    let conn = setup();
    let map = get_knowledge_embeddings_batch(&conn, &[]).unwrap();
    assert!(map.is_empty());
}
