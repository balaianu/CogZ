//! Integration tests for the search pipeline.
//!
//! Tests the full search flow: insert entities with embeddings and
//! edges, run hybrid search, verify ranking, graph expansion, and
//! provenance paths.

use cogz::config::SearchConfig;
use cogz::embed::{EmbeddingCache, EmbeddingModel, MockEmbeddingModel};
use cogz::search::{SearchMode, SearchParams, search};
use cogz::storage::{self, Storage, crud::Entity};

fn setup_storage() -> Storage {
    Storage::open_memory().unwrap()
}

fn default_config() -> SearchConfig {
    SearchConfig {
        fts_weight: 0.4,
        vec_weight: 0.6,
        rrf_k: 60,
        max_results: 20,
    }
}

fn insert_with_embedding(
    conn: &rusqlite::Connection,
    model: &dyn EmbeddingModel,
    id: &str,
    etype: &str,
    title: &str,
    content: &str,
) {
    let entity = Entity::new(id, etype, title, content);
    storage::crud::insert_entity(conn, &entity).unwrap();

    let text = format!("{title}\n\n{content}");
    let embeddings = model.embed(&[&text]).unwrap();
    storage::embeddings::insert_embedding(conn, id, &embeddings[0]).unwrap();
}

fn insert_edge(conn: &rusqlite::Connection, source: &str, target: &str, edge_type: &str) {
    let edge = storage::edges::Edge {
        source_id: source.to_string(),
        target_id: target.to_string(),
        edge_type: edge_type.to_string(),
        weight: 1.0,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    storage::edges::insert_edge(conn, &edge).unwrap();
}

#[test]
fn fts_only_search_finds_relevant_entities() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    insert_with_embedding(
        &conn,
        &model,
        "u1",
        "observation",
        "FTS5 ranking bug",
        "The RRF fusion produces incorrect rankings when k=60",
    );
    insert_with_embedding(
        &conn,
        &model,
        "u2",
        "rule",
        "Use SQLite",
        "Always use SQLite for local persistence",
    );
    insert_with_embedding(
        &conn,
        &model,
        "u3",
        "knowledge",
        "Search Architecture",
        "The search uses hybrid FTS and vector search",
    );

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

    assert_eq!(results.search_mode, SearchMode::FtsOnly);
    assert!(!results.results.is_empty());
    assert_eq!(results.results[0].entity.id, "u1");
}

#[test]
fn hybrid_search_fuses_fts_and_vector() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    // u1 matches both FTS ("ranking") and is close in vector space
    insert_with_embedding(
        &conn,
        &model,
        "u1",
        "observation",
        "ranking bug",
        "ranking content about FTS5",
    );
    // u2 matches FTS but is far in vector space
    insert_with_embedding(
        &conn,
        &model,
        "u2",
        "observation",
        "ranking issue",
        "ranking but different context entirely",
    );

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };

    // Query embedding uses the exact same text as u1's embedding
    let query_text = "ranking bug\n\nranking content about FTS5";
    let query_vec = model.embed(&[query_text]).unwrap();

    let results = search(
        &conn,
        "ranking",
        Some(&query_vec[0]),
        &params,
        &default_config(),
    )
    .unwrap();

    assert_eq!(results.search_mode, SearchMode::Hybrid);
    assert_eq!(results.results.len(), 2);
    // u1 should rank first (exact vec match + FTS match)
    assert_eq!(results.results[0].entity.id, "u1");
}

#[test]
fn graph_expansion_follows_edges() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    insert_with_embedding(
        &conn,
        &model,
        "obs1",
        "observation",
        "FTS5 bug",
        "ranking bug in search",
    );
    insert_with_embedding(
        &conn,
        &model,
        "func1",
        "function",
        "build_sql",
        "builds the FTS search SQL",
    );
    insert_with_embedding(
        &conn,
        &model,
        "func2",
        "function",
        "search_all",
        "executes the search query",
    );

    insert_edge(&conn, "obs1", "func1", "references");
    insert_edge(&conn, "func1", "func2", "calls");

    let params = SearchParams {
        expand: true,
        max_hops: 2,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

    // Direct match: obs1
    assert!(results.results.iter().any(|r| r.entity.id == "obs1"));

    // Expanded: func1 (1-hop via references)
    let func1 = results.results.iter().find(|r| r.entity.id == "func1");
    assert!(func1.is_some());
    let func1 = func1.unwrap();
    assert_eq!(func1.graph_path, vec!["obs1", "func1"]);
    assert!(!func1.graph_path_description.is_empty());

    // Expanded: func2 (2-hop via calls)
    let func2 = results.results.iter().find(|r| r.entity.id == "func2");
    assert!(func2.is_some());
    let func2 = func2.unwrap();
    assert_eq!(func2.graph_path, vec!["obs1", "func1", "func2"]);
}

#[test]
fn graph_expansion_disabled() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    insert_with_embedding(
        &conn,
        &model,
        "obs1",
        "observation",
        "FTS5 bug",
        "ranking bug",
    );
    insert_with_embedding(
        &conn,
        &model,
        "func1",
        "function",
        "build_sql",
        "sql builder",
    );
    insert_edge(&conn, "obs1", "func1", "references");

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

    // Only direct match, no expansion
    assert_eq!(results.results.len(), 1);
    assert_eq!(results.results[0].entity.id, "obs1");
}

#[test]
fn search_filters_by_entity_type() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    insert_with_embedding(
        &conn,
        &model,
        "u1",
        "observation",
        "ranking",
        "ranking content",
    );
    insert_with_embedding(
        &conn,
        &model,
        "u2",
        "rule",
        "ranking rule",
        "ranking content rule",
    );

    let params = SearchParams {
        entity_type: Some("rule".to_string()),
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

    assert_eq!(results.results.len(), 1);
    assert_eq!(results.results[0].entity.r#type, "rule");
}

#[test]
fn search_excludes_stale_by_default() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    let mut stale = Entity::new("u1", "observation", "ranking", "ranking content");
    stale.status = "stale".to_string();
    storage::crud::insert_entity(&conn, &stale).unwrap();
    let embeddings = model.embed(&["ranking\n\nranking content"]).unwrap();
    storage::embeddings::insert_embedding(&conn, "u1", &embeddings[0]).unwrap();

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();
    assert!(results.results.is_empty());
}

#[test]
fn search_includes_stale_with_status_all() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    let mut stale = Entity::new("u1", "observation", "ranking", "ranking content");
    stale.status = "stale".to_string();
    storage::crud::insert_entity(&conn, &stale).unwrap();
    let embeddings = model.embed(&["ranking\n\nranking content"]).unwrap();
    storage::embeddings::insert_embedding(&conn, "u1", &embeddings[0]).unwrap();

    let params = SearchParams {
        status: Some("all".to_string()),
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();
    assert_eq!(results.results.len(), 1);
}

#[test]
fn search_no_results_for_nonexistent_query() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    insert_with_embedding(
        &conn,
        &model,
        "u1",
        "observation",
        "Test",
        "some content here",
    );

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "nonexistent", None, &params, &default_config()).unwrap();
    assert!(results.results.is_empty());
}

#[test]
fn search_with_mock_embedding_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("observations")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("knowledge")).unwrap();

    let config_toml = cogz::config::default_toml("test-search");
    std::fs::write(cogz_dir.join("config.toml"), config_toml).unwrap();

    // Create entity files
    let obs = "---\nid: 550e8400-e29b-41d4-a716-446655440001\ntitle: \"FTS5 ranking bug\"\ntype: observation\nstatus: active\ncreated_at: 2026-08-28T10:00:00Z\nupdated_at: 2026-08-28T10:00:00Z\nreferences: [\"550e8400-e29b-41d4-a716-446655440002\"]\nsource: agent\n---\n\nThe RRF fusion produces incorrect rankings when k=60.\n";
    std::fs::write(cogz_dir.join("observations/test-obs.md"), obs).unwrap();

    let rule = "---\nid: 550e8400-e29b-41d4-a716-446655440002\ntitle: \"Use parameterized queries\"\ntype: rule\nstatus: active\ncreated_at: 2026-08-28T10:00:00Z\nupdated_at: 2026-08-28T10:00:00Z\nreferences: []\n---\n\nAlways use parameterized queries for FTS5 search.\n";
    std::fs::write(cogz_dir.join("rules/test-rule.md"), rule).unwrap();

    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = Storage::open(&db_path).unwrap();

    // Sync files to DB
    let sync_result = cogz::files::sync_all(&storage, &cogz_dir);
    assert_eq!(sync_result.created, 2);

    // Embed entities with mock model
    let model = MockEmbeddingModel::new();
    let cache = EmbeddingCache::new();
    {
        let conn = storage.conn();
        let entities: Vec<_> = sync_result
            .synced_entity_ids
            .iter()
            .filter_map(|id| storage::crud::get_entity(&conn, id).ok())
            .collect();
        let embeddings = cogz::files::embed_sync::embed_entities(&model, &cache, &entities);
        cogz::files::embed_sync::store_embeddings(&conn, &embeddings);
    }

    // Search
    let config = cogz::config::load(&cogz_dir.join("config.toml")).unwrap();
    let params = SearchParams {
        expand: true,
        max_hops: 2,
        ..Default::default()
    };

    let results = {
        let conn = storage.conn();
        search(&conn, "ranking", None, &params, &config.search).unwrap()
    };

    // Should find the observation
    assert!(!results.results.is_empty());
    let obs_result = results
        .results
        .iter()
        .find(|r| r.entity.title.as_deref() == Some("FTS5 ranking bug"));
    assert!(obs_result.is_some());

    // Graph expansion should find the rule (referenced by the observation)
    let rule_result = results
        .results
        .iter()
        .find(|r| r.entity.title.as_deref() == Some("Use parameterized queries"));
    assert!(rule_result.is_some());
    let rule_result = rule_result.unwrap();
    assert!(!rule_result.graph_path.is_empty());
}

#[test]
fn rrf_fusion_produces_sensible_ordering() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    // Three entities, all matching FTS for "ranking"
    insert_with_embedding(
        &conn,
        &model,
        "u1",
        "observation",
        "ranking first",
        "ranking content A",
    );
    insert_with_embedding(
        &conn,
        &model,
        "u2",
        "observation",
        "ranking second",
        "ranking content B",
    );
    insert_with_embedding(
        &conn,
        &model,
        "u3",
        "observation",
        "ranking third",
        "ranking content C",
    );

    let params = SearchParams {
        expand: false,
        ..Default::default()
    };

    // Query embedding matches u3's full text (title + content)
    let query_vec = model
        .embed(&["ranking third\n\nranking content C"])
        .unwrap();

    let results = search(
        &conn,
        "ranking",
        Some(&query_vec[0]),
        &params,
        &default_config(),
    )
    .unwrap();

    assert_eq!(results.search_mode, SearchMode::Hybrid);
    assert_eq!(results.results.len(), 3);

    // u3 should rank first: it's rank 1 in vec (exact match) and rank 3 in FTS.
    // u1 should rank second: it's rank 1 in FTS but rank 3 in vec.
    // With vec_weight=0.6 > fts_weight=0.4, u3 wins.
    assert_eq!(results.results[0].entity.id, "u3");
}

#[test]
fn search_limit_truncates_results() {
    let storage = setup_storage();
    let conn = storage.conn();
    let model = MockEmbeddingModel::new();

    for i in 0..10 {
        insert_with_embedding(
            &conn,
            &model,
            &format!("u{i}"),
            "observation",
            &format!("ranking {i}"),
            &format!("ranking content {i}"),
        );
    }

    let params = SearchParams {
        limit: 3,
        expand: false,
        ..Default::default()
    };
    let results = search(&conn, "ranking", None, &params, &default_config()).unwrap();

    assert_eq!(results.results.len(), 3);
}
