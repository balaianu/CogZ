//! Integration tests for Phase 11 hooks.
//!
//! Tests cover:
//! - session_start records event and returns cold_start context pack
//! - prompt_submit records event and returns task context pack
//! - pre_tool_use records event, no context pack, no observation
//! - post_tool_use records event and observation when both fields present
//! - post_tool_use records event only when fields missing
//! - Events are queryable from the DB
//! - Invalid event type returns error

use std::sync::Arc;

use cogz::config::Config;
use cogz::embed::{ModelType, OnnxEmbeddingModel};
use cogz::hooks::lifecycle::{LifecycleEvent, LifecycleInput, handle_lifecycle_event};
use cogz::storage::Storage;
use cogz::storage::events::get_recent_events;

fn setup() -> (Arc<Storage>, Config, std::path::PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(&cogz_dir).unwrap();
    std::fs::create_dir_all(cogz_dir.join("observations")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("knowledge")).unwrap();

    let storage = Arc::new(Storage::open_memory().unwrap());
    let config = Config::default_for("test-hooks");
    (storage, config, cogz_dir, dir)
}

fn query_model(config: &Config) -> OnnxEmbeddingModel {
    let models_dir = cogz::embed::models_dir();
    OnnxEmbeddingModel::with_model_id(
        ModelType::Knowledge,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.knowledge_model,
    )
}

fn code_model(config: &Config) -> OnnxEmbeddingModel {
    let models_dir = cogz::embed::models_dir();
    OnnxEmbeddingModel::with_model_id(
        ModelType::Code,
        &models_dir,
        config.embedding.dimension,
        &config.embedding.code_model,
    )
}

#[test]
fn session_start_records_event_and_returns_cold_start_pack() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::SessionStart,
            prompt: None,
            tool_name: None,
            tool_result: None,
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    let pack = output
        .context_pack
        .expect("session_start should return a pack");
    assert_eq!(pack.mode, cogz::context::ContextMode::ColdStart);
    assert!(output.observation_id.is_none());

    let conn = storage.conn();
    let events = get_recent_events(&conn, "session_start", 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "session_start");
}

#[test]
fn prompt_submit_records_event_and_returns_task_pack() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    // Need some entities for task mode to find.
    insert_test_observation(&storage, "Test knowledge", "Some content about testing");

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::PromptSubmit,
            prompt: Some("testing"),
            tool_name: None,
            tool_result: None,
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    let pack = output
        .context_pack
        .expect("prompt_submit should return a pack");
    assert_eq!(pack.mode, cogz::context::ContextMode::Task);
    assert_eq!(pack.query, "testing");
    assert!(output.observation_id.is_none());

    let conn = storage.conn();
    let events = get_recent_events(&conn, "prompt_submit", 10).unwrap();
    assert_eq!(events.len(), 1);
}

#[test]
fn pre_tool_use_records_event_only() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::PreToolUse,
            prompt: None,
            tool_name: Some("edit_file"),
            tool_result: None,
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    assert!(
        output.context_pack.is_none(),
        "pre_tool_use should not return a pack"
    );
    assert!(
        output.observation_id.is_none(),
        "pre_tool_use should not record an observation"
    );

    let conn = storage.conn();
    let events = get_recent_events(&conn, "pre_tool_use", 10).unwrap();
    assert_eq!(events.len(), 1);
}

#[test]
fn post_tool_use_records_event_but_not_observation() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::PostToolUse,
            prompt: None,
            tool_name: Some("run_tests"),
            tool_result: Some("329 passed, 0 failed"),
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    assert!(output.context_pack.is_none());
    // post_tool_use no longer auto-records observations — the agent
    // decides what's salient via the record_observation MCP tool.
    assert!(
        output.observation_id.is_none(),
        "post_tool_use should not auto-record an observation"
    );

    // No observation file should be created.
    let obs_dir = cogz_dir.join("observations");
    if obs_dir.exists() {
        let obs_files: Vec<_> = std::fs::read_dir(&obs_dir)
            .unwrap()
            .flatten()
            .flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten())
            .collect();
        assert!(obs_files.is_empty(), "no observation files should exist");
    }

    // The event should still be recorded.
    let conn = storage.conn();
    let events = get_recent_events(&conn, "post_tool_use", 10).unwrap();
    assert_eq!(events.len(), 1);
}

#[test]
fn post_tool_use_without_tool_result_records_event_only() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::PostToolUse,
            prompt: None,
            tool_name: Some("edit_file"),
            tool_result: None,
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    assert!(
        output.observation_id.is_none(),
        "post_tool_use should not auto-record an observation"
    );
}

#[test]
fn lifecycle_event_parse_roundtrip() {
    assert_eq!(
        LifecycleEvent::parse("session_start"),
        Some(LifecycleEvent::SessionStart)
    );
    assert_eq!(
        LifecycleEvent::parse("prompt_submit"),
        Some(LifecycleEvent::PromptSubmit)
    );
    assert_eq!(
        LifecycleEvent::parse("pre_tool_use"),
        Some(LifecycleEvent::PreToolUse)
    );
    assert_eq!(
        LifecycleEvent::parse("post_tool_use"),
        Some(LifecycleEvent::PostToolUse)
    );
    assert_eq!(
        LifecycleEvent::parse("file_save"),
        Some(LifecycleEvent::FileSave)
    );
    assert_eq!(
        LifecycleEvent::parse("session_end"),
        Some(LifecycleEvent::SessionEnd)
    );
    assert_eq!(LifecycleEvent::parse("invalid"), None);
}

#[test]
fn file_save_for_cogz_file_triggers_sync_not_reindex() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::FileSave,
            prompt: None,
            tool_name: None,
            tool_result: None,
            file_path: Some(".cogz/knowledge/test.md"),
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    assert!(output.context_pack.is_none());
    assert!(output.observation_id.is_none());
    let summary = output
        .reindex_summary
        .expect("file_save should return a summary");
    assert!(
        !summary.reindexed,
        ".cogz/ files should not trigger code reindex"
    );
    assert!(summary.synced, ".cogz/ files should trigger file sync");

    let conn = storage.conn();
    let events = get_recent_events(&conn, "file_save", 10).unwrap();
    assert_eq!(events.len(), 1);
}

#[test]
fn file_save_without_path_records_event_only() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::FileSave,
            prompt: None,
            tool_name: None,
            tool_result: None,
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    let summary = output
        .reindex_summary
        .expect("file_save should return a summary");
    assert!(!summary.reindexed);
    assert!(!summary.synced);
}

#[test]
fn session_end_records_event_and_runs_consolidation() {
    let (storage, config, cogz_dir, _dir) = setup();
    let model = query_model(&config);
    let code_mod = code_model(&config);

    let output = handle_lifecycle_event(
        &storage,
        &config,
        &cogz_dir,
        &model,
        &code_mod,
        None,
        &LifecycleInput {
            event: LifecycleEvent::SessionEnd,
            prompt: None,
            tool_name: None,
            tool_result: None,
            file_path: None,
        },
    )
    .unwrap();

    assert!(output.event_id > 0);
    assert!(output.context_pack.is_none());
    assert!(output.observation_id.is_none());
    assert!(
        output.reindex_summary.is_none(),
        "session_end should not trigger reindex"
    );
    let summary = output
        .consolidation_summary
        .expect("session_end should return a consolidation summary");
    // No entities meet promotion/merge thresholds, but consolidation runs.
    assert_eq!(summary.promotions, 0);
    assert_eq!(summary.merges, 0);

    let conn = storage.conn();
    let events = get_recent_events(&conn, "session_end", 10).unwrap();
    assert_eq!(events.len(), 1);
}

fn insert_test_observation(storage: &Storage, title: &str, content: &str) {
    use cogz::storage::crud::{Entity, insert_entity};
    let entity = Entity::new(
        &uuid::Uuid::new_v4().to_string(),
        "observation",
        title,
        content,
    );
    let conn = storage.conn();
    insert_entity(&conn, &entity).unwrap();
}
