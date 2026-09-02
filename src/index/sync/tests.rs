use super::*;
use crate::index::tree_sitter::{Language, extract_all};
use crate::storage::Storage;

/// Helper: parse source files and return entities_by_file for sync.
fn parse_files(
    files: &[(std::path::PathBuf, String, Language)],
) -> Vec<(String, Vec<crate::index::tree_sitter::CodeEntity>)> {
    files
        .iter()
        .map(|(path, source, lang)| {
            let (entities, _) = extract_all(path, source, *lang);
            (path.to_string_lossy().to_string(), entities)
        })
        .collect()
}

#[test]
fn uuid_is_deterministic() {
    let id1 = code_entity_uuid("src/main.rs", "function", "main");
    let id2 = code_entity_uuid("src/main.rs", "function", "main");
    assert_eq!(id1, id2);
}

#[test]
fn uuid_differs_by_entity() {
    let id1 = code_entity_uuid("src/main.rs", "function", "main");
    let id2 = code_entity_uuid("src/main.rs", "function", "helper");
    assert_ne!(id1, id2);
}

#[test]
fn uuid_differs_by_file() {
    let id1 = code_entity_uuid("src/main.rs", "function", "main");
    let id2 = code_entity_uuid("src/lib.rs", "function", "main");
    assert_ne!(id1, id2);
}

#[test]
fn uuid_differs_by_type() {
    let id1 = code_entity_uuid("src/main.rs", "function", "foo");
    let id2 = code_entity_uuid("src/main.rs", "class", "foo");
    assert_ne!(id1, id2);
}

#[test]
fn sync_inserts_code_entities() {
    let storage = Storage::open_memory().unwrap();
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    let entities = parse_files(&files);
    let result = sync_code_entities(&storage, &entities);
    assert_eq!(result.created, 2); // file + function
    assert_eq!(result.updated, 0);
    assert_eq!(result.marked_stale, 0);

    let conn = storage.conn();
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 2);
}

#[test]
fn sync_skips_unchanged_entities() {
    let storage = Storage::open_memory().unwrap();
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // First sync — creates
    let entities = parse_files(&files);
    let result1 = sync_code_entities(&storage, &entities);
    assert_eq!(result1.created, 2);

    // Second sync — skips (same content)
    let result2 = sync_code_entities(&storage, &entities);
    assert_eq!(result2.created, 0);
    assert_eq!(result2.skipped, 2);
}

#[test]
fn sync_updates_changed_entities() {
    let storage = Storage::open_memory().unwrap();
    let code1 = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let code2 = "fn add(a: i32, b: i32) -> i64 { (a + b) as i64 }";
    let files1 = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code1.to_string(),
        Language::Rust,
    )];

    // First sync
    let entities1 = parse_files(&files1);
    sync_code_entities(&storage, &entities1);

    // Second sync with changed content
    let files2 = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code2.to_string(),
        Language::Rust,
    )];
    let entities2 = parse_files(&files2);
    let result2 = sync_code_entities(&storage, &entities2);
    assert_eq!(result2.updated, 2);
    assert_eq!(result2.skipped, 0);
}

#[test]
fn sync_marks_stale_when_file_removed() {
    let storage = Storage::open_memory().unwrap();
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // First sync — creates
    let entities = parse_files(&files);
    sync_code_entities(&storage, &entities);

    // Second sync — empty file list, should mark stale
    let result = sync_code_entities(&storage, &[]);
    assert_eq!(result.marked_stale, 2);
}

#[test]
fn sync_reactivates_stale_entities() {
    let storage = Storage::open_memory().unwrap();
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // First sync — creates
    let entities = parse_files(&files);
    sync_code_entities(&storage, &entities);

    // Second sync — empty, marks stale
    sync_code_entities(&storage, &[]);

    // Third sync — file is back, should reactivate
    let result = sync_code_entities(&storage, &entities);
    assert_eq!(result.updated, 2);
    assert_eq!(result.marked_stale, 0);

    // Verify status is active again
    let conn = storage.conn();
    let active_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 2);
}

#[test]
fn rebuild_produces_same_uuids() {
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // First sync
    let storage = Storage::open_memory().unwrap();
    let entities = parse_files(&files);
    sync_code_entities(&storage, &entities);
    let conn = storage.conn();
    let ids_before: Vec<String> = conn
        .prepare("SELECT id FROM entities ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // Reset and rebuild
    drop(conn);
    let storage2 = Storage::open_memory().unwrap();
    sync_code_entities(&storage2, &entities);
    let conn2 = storage2.conn();
    let ids_after: Vec<String> = conn2
        .prepare("SELECT id FROM entities ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert_eq!(ids_before, ids_after);
}

#[test]
fn incremental_sync_marks_renamed_entities_stale() {
    let storage = Storage::open_memory().unwrap();

    // V1: file defines `old_fn`
    let code_v1 = "fn old_fn() {}\nfn keeper() {}\n";
    let files_v1 = vec![(
        std::path::PathBuf::from("src/test.rs"),
        code_v1.to_string(),
        Language::Rust,
    )];
    let entities_v1 = parse_files(&files_v1);
    sync_code_entities(&storage, &entities_v1);

    let old_fn_id = code_entity_uuid("src/test.rs", "function", "old_fn");
    let keeper_id = code_entity_uuid("src/test.rs", "function", "keeper");
    {
        let conn = storage.conn();
        let old_status: String = conn
            .query_row(
                "SELECT status FROM entities WHERE id = ?1",
                rusqlite::params![old_fn_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_status, "active");
    }

    // V2: `old_fn` renamed to `new_fn`, `keeper` unchanged
    let code_v2 = "fn new_fn() {}\nfn keeper() {}\n";
    let files_v2 = vec![(
        std::path::PathBuf::from("src/test.rs"),
        code_v2.to_string(),
        Language::Rust,
    )];
    let entities_v2 = parse_files(&files_v2);
    let result = sync_code_entities_incremental(&storage, &entities_v2);

    // old_fn should be marked stale (not left active)
    let conn = storage.conn();
    let old_status: String = conn
        .query_row(
            "SELECT status FROM entities WHERE id = ?1",
            rusqlite::params![old_fn_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        old_status, "stale",
        "renamed entity should be marked stale, not left active"
    );

    // new_fn should be active
    let new_fn_id = code_entity_uuid("src/test.rs", "function", "new_fn");
    let new_status: String = conn
        .query_row(
            "SELECT status FROM entities WHERE id = ?1",
            rusqlite::params![new_fn_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new_status, "active");

    // keeper should still be active (unchanged)
    let keeper_status: String = conn
        .query_row(
            "SELECT status FROM entities WHERE id = ?1",
            rusqlite::params![keeper_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(keeper_status, "active");

    // The stale-marked ID should be in the result for knowledge flagging
    assert!(
        result.removed_entity_ids.contains(&old_fn_id),
        "removed_entity_ids should contain the renamed entity's ID"
    );
}
