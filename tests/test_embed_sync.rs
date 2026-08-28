//! Integration tests for embedding integration with file sync.
//!
//! Tests that the sync pipeline correctly embeds entities using the
//! mock model, stores vectors in vec0, and degrades gracefully when
//! the model is unavailable.

use std::fs;

use cogz::embed::{EmbeddingCache, EmbeddingModel, MockEmbeddingModel};
use cogz::files::embed_sync::embed_synced_entities;
use cogz::storage::{self, crud::Entity};

fn setup_storage(dir: &std::path::Path) -> storage::Storage {
    let db_path = dir.join(".cogz/cogz.db");
    storage::Storage::open(&db_path).unwrap()
}

fn make_entity(id: &str, etype: &str, title: &str, content: &str) -> Entity {
    Entity::new(id, etype, title, content)
}

#[test]
fn embed_synced_entities_stores_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let storage = setup_storage(dir.path());
    let conn = storage.conn();

    let e1 = make_entity("entity-1", "observation", "Test A", "content A");
    let e2 = make_entity("entity-2", "knowledge", "Test B", "content B");
    storage::crud::insert_entity(&conn, &e1).unwrap();
    storage::crud::insert_entity(&conn, &e2).unwrap();

    let model = MockEmbeddingModel::new();
    let cache = EmbeddingCache::new();

    let embedded = embed_synced_entities(&conn, &model, &cache, &[e1, e2]);

    assert_eq!(embedded, 2);
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 2);
}

#[test]
fn embed_synced_entities_skips_when_model_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let storage = setup_storage(dir.path());
    let conn = storage.conn();

    let entity = make_entity("entity-1", "observation", "Test", "content");
    storage::crud::insert_entity(&conn, &entity).unwrap();

    // OnnxEmbeddingModel without model files — is_available() is false
    let models_dir = dir.path().join("models");
    let model =
        cogz::embed::OnnxEmbeddingModel::new(cogz::embed::ModelType::Knowledge, &models_dir, 768);

    let cache = EmbeddingCache::new();
    let embedded = embed_synced_entities(&conn, &model, &cache, &[entity]);

    assert_eq!(embedded, 0);
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 0);
}

#[test]
fn embed_synced_entities_uses_cache_on_second_call() {
    let dir = tempfile::tempdir().unwrap();
    let storage = setup_storage(dir.path());
    let conn = storage.conn();

    let entity = make_entity("entity-1", "observation", "Test", "content");
    storage::crud::insert_entity(&conn, &entity).unwrap();

    let model = MockEmbeddingModel::new();
    let cache = EmbeddingCache::new();

    // First embed — miss, computes and stores
    let embedded1 = embed_synced_entities(&conn, &model, &cache, std::slice::from_ref(&entity));
    assert_eq!(embedded1, 1);
    assert_eq!(cache.misses(), 1);
    assert_eq!(cache.hits(), 0);

    // Delete the embedding from DB to verify re-storage
    storage::embeddings::delete_embedding(&conn, "entity-1").unwrap();
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 0);

    // Second embed — cache hit, still stores in DB
    let embedded2 = embed_synced_entities(&conn, &model, &cache, std::slice::from_ref(&entity));
    assert_eq!(embedded2, 1);
    assert_eq!(cache.hits(), 1);
    assert_eq!(cache.misses(), 1);
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 1);
}

#[test]
fn embed_synced_entities_handles_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let storage = setup_storage(dir.path());
    let conn = storage.conn();

    let model = MockEmbeddingModel::new();
    let cache = EmbeddingCache::new();

    let embedded = embed_synced_entities(&conn, &model, &cache, &[]);
    assert_eq!(embedded, 0);
}

#[test]
fn embed_synced_entities_replaces_existing_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let storage = setup_storage(dir.path());
    let conn = storage.conn();

    let entity = make_entity("entity-1", "knowledge", "Test", "content");
    storage::crud::insert_entity(&conn, &entity).unwrap();

    // Pre-insert a dummy embedding
    let dummy = vec![0.5_f32; 768];
    storage::embeddings::insert_embedding(&conn, "entity-1", &dummy).unwrap();
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 1);

    // Embed with mock model — should replace the dummy
    let model = MockEmbeddingModel::new();
    let cache = EmbeddingCache::new();
    let embedded = embed_synced_entities(&conn, &model, &cache, &[entity]);

    assert_eq!(embedded, 1);
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 1);

    // Verify the embedding changed by doing a KNN search
    let results = storage::embeddings::knn_search(&conn, &dummy, 1).unwrap();
    // The new embedding should be different from dummy, so distance > 0
    assert!(!results.is_empty());
    assert!(results[0].1 > 0.01);
}

#[test]
fn full_sync_with_mock_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    fs::create_dir_all(cogz_dir.join("observations")).unwrap();
    fs::create_dir_all(cogz_dir.join("rules")).unwrap();
    fs::create_dir_all(cogz_dir.join("knowledge")).unwrap();

    // Write a config
    let config = cogz::config::default_toml("test-project");
    fs::write(cogz_dir.join("config.toml"), config).unwrap();

    // Write an observation file
    let obs_content = "---\nid: 550e8400-e29b-41d4-a716-446655440000\ntitle: Test Observation\ntype: observation\nstatus: active\ncreated_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\nThis is a test observation about Rust.\n";
    fs::write(cogz_dir.join("observations/test-obs.md"), obs_content).unwrap();

    // Sync files to DB
    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = storage::Storage::open(&db_path).unwrap();
    let result = cogz::files::sync_all(&storage, &cogz_dir);

    assert_eq!(result.created, 1);
    assert_eq!(result.synced_entity_ids.len(), 1);

    // Embed using mock model
    let conn = storage.conn();
    let entity_id = &result.synced_entity_ids[0];
    let entity = storage::crud::get_entity(&conn, entity_id).unwrap();

    let model = MockEmbeddingModel::new();
    let cache = EmbeddingCache::new();
    let embedded = embed_synced_entities(&conn, &model, &cache, &[entity]);

    assert_eq!(embedded, 1);
    assert_eq!(storage::embeddings::count_embeddings(&conn).unwrap(), 1);

    // Verify the embedding is searchable
    let query = model
        .embed(&["This is a test observation about Rust."])
        .unwrap();
    let results = storage::embeddings::knn_search(&conn, &query[0], 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, *entity_id);
}
