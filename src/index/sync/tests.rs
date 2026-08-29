use super::*;
use crate::storage::Storage;

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

    let result = sync_code_entities(&storage, Path::new("."), &files);
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
    let result1 = sync_code_entities(&storage, Path::new("."), &files);
    assert_eq!(result1.created, 2);

    // Second sync — skips (same content)
    let result2 = sync_code_entities(&storage, Path::new("."), &files);
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
    sync_code_entities(&storage, Path::new("."), &files1);

    // Second sync with changed content
    let files2 = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code2.to_string(),
        Language::Rust,
    )];
    let result2 = sync_code_entities(&storage, Path::new("."), &files2);
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
    sync_code_entities(&storage, Path::new("."), &files);

    // Second sync — empty file list, should mark stale
    let result = sync_code_entities(&storage, Path::new("."), &[]);
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
    sync_code_entities(&storage, Path::new("."), &files);

    // Second sync — empty, marks stale
    sync_code_entities(&storage, Path::new("."), &[]);

    // Third sync — file is back, should reactivate
    let result = sync_code_entities(&storage, Path::new("."), &files);
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
    let storage = Storage::open_memory().unwrap();
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // First sync
    sync_code_entities(&storage, Path::new("."), &files);
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
    sync_code_entities(&storage2, Path::new("."), &files);
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
