//! Integration tests for Phase 10: git diff + stale flagging.
//!
//! Tests the full flow: index a git repo with source + observation,
//! modify the source, reindex, verify stale flagging on the observation
//! and the code_changed event.

use std::fs;
use std::path::Path;

use cogz::config::Config;
use cogz::files::{EntityFile, FileEntityType, write_entity_file};
use cogz::index::{self, stale_flagging};
use cogz::storage::{
    self, Storage,
    crud::{Entity, insert_entity},
    edges::{Edge, insert_edge},
};

fn init_git_repo(dir: &Path) {
    git2::Repository::init(dir).unwrap();
    let repo = git2::Repository::open(dir).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "test").unwrap();
    config.set_str("user.email", "test@test.com").unwrap();
}

fn commit_all(dir: &Path) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();

    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();

    let sig = repo.signature().unwrap();
    let head = repo.head().ok();
    let parents: Vec<_> = head
        .iter()
        .filter_map(|h| h.peel_to_commit().ok())
        .collect();
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    let commit = repo
        .commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parent_refs)
        .unwrap();
    commit.to_string()
}

fn write_file(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn default_config() -> Config {
    Config::default_for("test-project")
}

#[test]
fn reindex_detects_modified_source_and_flags_stale_knowledge() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());

    // Create a source file and an observation referencing a function.
    write_file(
        dir.path(),
        "src/lib.rs",
        "pub fn greet() -> &'static str { \"hello\" }",
    );
    write_file(
        dir.path(),
        ".cogz/config.toml",
        &cogz::config::default_toml("test-project"),
    );
    fs::create_dir_all(dir.path().join(".cogz/observations/2026-08")).unwrap();

    let sha = commit_all(dir.path());

    // Index.
    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = Storage::open(&db_path, 768).unwrap();
    let config = default_config();
    index::index_code(&storage, dir.path(), &config);

    // Store baseline commit.
    {
        let conn = storage.conn();
        storage::set_meta(&conn, "last_indexed_commit", &sha).unwrap();
    }

    // Find the greet function's code entity ID.
    let conn = storage.conn();
    let functions =
        storage::query::get_entities_by_type(&conn, "function", Some("active"), 100).unwrap();
    let greet = functions
        .iter()
        .find(|f| f.title.as_deref() == Some("greet"))
        .unwrap();
    let greet_id = greet.id.clone();

    // Create an observation that references greet.
    let mut obs = EntityFile::new(
        "Bug in greet",
        FileEntityType::Observation,
        "greet returns wrong value",
    );
    obs.id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string();
    obs.references = vec![greet_id.clone()];
    let obs_path = obs.file_path(&dir.path().join(".cogz"));
    if let Some(parent) = obs_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    write_entity_file(&obs_path, &obs).unwrap();

    // Insert observation into DB directly (file is already written).
    let mut obs_entity = Entity::new(
        &obs.id,
        "observation",
        "Bug in greet",
        "greet returns wrong value",
    );
    obs_entity.file_path = Some(
        obs_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string(),
    );
    insert_entity(&conn, &obs_entity).unwrap();
    insert_edge(
        &conn,
        &Edge {
            source_id: obs.id.clone(),
            target_id: greet_id.clone(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();

    // Verify observation is active.
    let obs_entity = storage::crud::get_entity(&conn, &obs.id).unwrap();
    assert_eq!(obs_entity.status, "active");
    drop(conn);

    // Modify the source file.
    write_file(
        dir.path(),
        "src/lib.rs",
        "pub fn greet() -> &'static str { \"hi\" }",
    );

    // Reindex.
    let result = index::reindex_code(&storage, dir.path(), &config);
    assert!(result.incremental);
    assert!(result.updated >= 1, "expected at least 1 updated entity");

    // Flag stale knowledge.
    let all_changed = result.changed_code_ids;
    assert!(!all_changed.is_empty(), "expected changed code IDs");

    let flagged =
        stale_flagging::flag_stale_knowledge(&storage, &dir.path().join(".cogz"), &all_changed);
    assert!(flagged >= 1, "expected at least 1 stale knowledge flagged");

    // Verify observation is now stale.
    let conn = storage.conn();
    let obs_entity = storage::crud::get_entity(&conn, &obs.id).unwrap();
    assert_eq!(obs_entity.status, "stale");

    // Verify code_changed event was recorded.
    let events = storage::events::get_recent_events(&conn, "code_changed", 10).unwrap();
    assert!(!events.is_empty(), "expected code_changed event");
    drop(conn);
}

#[test]
fn reindex_no_changes_is_fast_noop() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());

    write_file(dir.path(), "src/lib.rs", "pub fn foo() {}");
    write_file(
        dir.path(),
        ".cogz/config.toml",
        &cogz::config::default_toml("test-project"),
    );
    let sha = commit_all(dir.path());

    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = Storage::open(&db_path, 768).unwrap();
    let config = default_config();
    index::index_code(&storage, dir.path(), &config);

    {
        let conn = storage.conn();
        storage::set_meta(&conn, "last_indexed_commit", &sha).unwrap();
    }

    // Reindex without any changes.
    let result = index::reindex_code(&storage, dir.path(), &config);
    assert!(result.incremental);
    assert_eq!(result.created, 0);
    assert_eq!(result.updated, 0);
    assert_eq!(result.marked_stale, 0);
}

#[test]
fn reindex_falls_back_to_full_scan_without_git() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/lib.rs", "pub fn foo() {}");
    write_file(
        dir.path(),
        ".cogz/config.toml",
        &cogz::config::default_toml("test-project"),
    );

    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = Storage::open(&db_path, 768).unwrap();
    let config = default_config();

    // No git repo, no baseline — should fall back to full scan.
    let result = index::reindex_code(&storage, dir.path(), &config);
    assert!(!result.incremental);
    assert!(
        result.created > 0,
        "expected entities created from full scan"
    );
}

#[test]
fn deleted_source_marks_code_stale_and_flags_knowledge() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());

    write_file(dir.path(), "src/lib.rs", "pub fn keep() {}");
    write_file(dir.path(), "src/gone.rs", "pub fn remove_me() {}");
    write_file(
        dir.path(),
        ".cogz/config.toml",
        &cogz::config::default_toml("test-project"),
    );
    fs::create_dir_all(dir.path().join(".cogz/observations/2026-08")).unwrap();
    let sha = commit_all(dir.path());

    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = Storage::open(&db_path, 768).unwrap();
    let config = default_config();
    index::index_code(&storage, dir.path(), &config);

    {
        let conn = storage.conn();
        storage::set_meta(&conn, "last_indexed_commit", &sha).unwrap();
    }

    // Find remove_me function.
    let conn = storage.conn();
    let functions =
        storage::query::get_entities_by_type(&conn, "function", Some("active"), 100).unwrap();
    let remove_me = functions
        .iter()
        .find(|f| f.title.as_deref() == Some("remove_me"))
        .unwrap();
    let remove_me_id = remove_me.id.clone();

    // Create an observation referencing remove_me.
    let mut obs = EntityFile::new(
        "Note about remove_me",
        FileEntityType::Observation,
        "remove_me does X",
    );
    obs.id = "11111111-2222-3333-4444-555555555555".to_string();
    obs.references = vec![remove_me_id.clone()];
    let obs_path = obs.file_path(&dir.path().join(".cogz"));
    if let Some(parent) = obs_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    write_entity_file(&obs_path, &obs).unwrap();

    // Insert observation into DB directly.
    let mut obs_entity = Entity::new(
        &obs.id,
        "observation",
        "Note about remove_me",
        "remove_me does X",
    );
    obs_entity.file_path = Some(
        obs_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string(),
    );
    insert_entity(&conn, &obs_entity).unwrap();
    insert_edge(
        &conn,
        &Edge {
            source_id: obs.id.clone(),
            target_id: remove_me_id.clone(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    drop(conn);

    // Delete the source file.
    fs::remove_file(dir.path().join("src/gone.rs")).unwrap();

    // Reindex.
    let result = index::reindex_code(&storage, dir.path(), &config);
    assert!(result.incremental);
    assert!(
        result.marked_stale > 0,
        "expected stale code entities from deleted file"
    );
    assert!(
        !result.deleted_code_ids.is_empty(),
        "expected deleted code IDs"
    );

    // Flag stale knowledge.
    let all_changed: Vec<String> = result
        .changed_code_ids
        .iter()
        .chain(result.deleted_code_ids.iter())
        .cloned()
        .collect();
    let flagged =
        stale_flagging::flag_stale_knowledge(&storage, &dir.path().join(".cogz"), &all_changed);
    assert!(flagged >= 1, "expected observation flagged stale");

    // Verify observation is stale.
    let conn = storage.conn();
    let obs_entity = storage::crud::get_entity(&conn, &obs.id).unwrap();
    assert_eq!(obs_entity.status, "stale");
    drop(conn);
}

#[test]
fn unchanged_files_not_re_parsed() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());

    write_file(dir.path(), "src/a.rs", "pub fn a() {}");
    write_file(dir.path(), "src/b.rs", "pub fn b() {}");
    write_file(
        dir.path(),
        ".cogz/config.toml",
        &cogz::config::default_toml("test-project"),
    );
    let sha = commit_all(dir.path());

    let db_path = dir.path().join(".cogz/cogz.db");
    let storage = Storage::open(&db_path, 768).unwrap();
    let config = default_config();
    index::index_code(&storage, dir.path(), &config);

    {
        let conn = storage.conn();
        storage::set_meta(&conn, "last_indexed_commit", &sha).unwrap();
    }

    // Only modify a.rs.
    write_file(dir.path(), "src/a.rs", "pub fn a() -> i32 { 42 }");

    let result = index::reindex_code(&storage, dir.path(), &config);
    assert!(result.incremental);
    // Only a.rs should be parsed — b.rs is unchanged.
    // The sync result should show 0 skipped (b.rs wasn't even parsed).
    assert!(result.updated >= 1, "expected a.rs entities updated");
    assert_eq!(
        result.skipped, 0,
        "unchanged files should not be parsed at all"
    );
}
