use super::*;
use crate::files::{EntityFile, FileEntityType, write_entity_file};
use crate::storage::crud::{Entity, insert_entity};
use crate::storage::edges::{Edge, insert_edge};
use tempfile::TempDir;

fn setup() -> (TempDir, Storage) {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("observations/2026-08")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("knowledge/test")).unwrap();

    let db_path = cogz_dir.join("cogz.db");
    let storage = Storage::open(&db_path, 768).unwrap();
    (dir, storage)
}

fn make_observation(cogz_dir: &Path, id: &str, title: &str, content: &str) -> EntityFile {
    let mut entity = EntityFile::new(title, FileEntityType::Observation, content);
    entity.id = id.to_string();
    entity.status = "active".to_string();
    let path = entity.file_path(cogz_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    write_entity_file(&path, &entity).unwrap();
    entity
}

fn make_db_observation(id: &str, title: &str, content: &str) -> Entity {
    let year_month = chrono::Utc::now().format("%Y-%m").to_string();
    let mut entity = Entity::new(id, "observation", title, content);
    entity.file_path = Some(format!("observations/{year_month}/{id}.md"));
    entity
}

#[test]
fn flags_observation_referencing_changed_code() {
    let (dir, storage) = setup();
    let cogz_dir = dir.path().join(".cogz");

    // Insert a code entity (function).
    let conn = storage.conn();
    let code_entity = Entity::new("code-uuid-1", "function", "my_func", "fn my_func() {}");
    insert_entity(&conn, &code_entity).unwrap();

    // Insert an observation that references the code entity.
    let obs = make_observation(
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000021",
        "Bug in my_func",
        "Found a bug",
    );
    let obs_entity = make_db_observation(
        "a1b2c3d4-e5f6-4789-abcd-000000000021",
        "Bug in my_func",
        "Found a bug",
    );
    insert_entity(&conn, &obs_entity).unwrap();
    insert_edge(
        &conn,
        &Edge {
            source_id: "a1b2c3d4-e5f6-4789-abcd-000000000021".to_string(),
            target_id: "code-uuid-1".to_string(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    drop(conn);

    // Flag stale.
    let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-1".to_string()]);
    assert_eq!(count, 1);

    // Verify DB status.
    let conn = storage.conn();
    let entity = storage::crud::get_entity(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000021").unwrap();
    assert_eq!(entity.status, "stale");

    // Verify file frontmatter.
    let path = obs.file_path(&cogz_dir);
    let file = read_entity_file(&path).unwrap();
    assert_eq!(file.status, "stale");

    // Verify event was recorded.
    let events = storage::events::get_recent_events(&conn, "code_changed", 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["count"], 1);
}

#[test]
fn does_not_flag_already_stale() {
    let (dir, storage) = setup();
    let cogz_dir = dir.path().join(".cogz");

    let conn = storage.conn();
    let code_entity = Entity::new("code-uuid-2", "function", "func2", "fn func2() {}");
    insert_entity(&conn, &code_entity).unwrap();

    let mut obs = make_observation(
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000022",
        "Note about func2",
        "Some note",
    );
    obs.status = "stale".to_string();
    let path = obs.file_path(&cogz_dir);
    write_entity_file(&path, &obs).unwrap();

    let obs_entity = make_db_observation(
        "a1b2c3d4-e5f6-4789-abcd-000000000022",
        "Note about func2",
        "Some note",
    );
    let mut db_entity = obs_entity.clone();
    db_entity.status = "stale".to_string();
    insert_entity(&conn, &db_entity).unwrap();
    insert_edge(
        &conn,
        &Edge {
            source_id: "a1b2c3d4-e5f6-4789-abcd-000000000022".to_string(),
            target_id: "code-uuid-2".to_string(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    drop(conn);

    let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-2".to_string()]);
    assert_eq!(count, 0);
}

#[test]
fn does_not_flag_unrelated_entities() {
    let (dir, storage) = setup();
    let cogz_dir = dir.path().join(".cogz");

    let conn = storage.conn();
    let code_entity = Entity::new("code-uuid-3", "function", "func3", "fn func3() {}");
    insert_entity(&conn, &code_entity).unwrap();

    // Observation that does NOT reference the code entity.
    make_observation(
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000024",
        "Unrelated note",
        "Nothing about func3",
    );
    let obs_entity = make_db_observation(
        "a1b2c3d4-e5f6-4789-abcd-000000000024",
        "Unrelated note",
        "Nothing about func3",
    );
    insert_entity(&conn, &obs_entity).unwrap();
    drop(conn);

    let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-3".to_string()]);
    assert_eq!(count, 0);
}

#[test]
fn empty_changed_ids_returns_zero() {
    let (dir, storage) = setup();
    let cogz_dir = dir.path().join(".cogz");

    let count = flag_stale_knowledge(&storage, &cogz_dir, &[]);
    assert_eq!(count, 0);
}

#[test]
fn flags_multiple_referencing_entities() {
    let (dir, storage) = setup();
    let cogz_dir = dir.path().join(".cogz");

    let conn = storage.conn();
    let code_entity = Entity::new(
        "code-uuid-4",
        "function",
        "shared_func",
        "fn shared_func() {}",
    );
    insert_entity(&conn, &code_entity).unwrap();

    // Two observations referencing the same code entity.
    for i in 0..2 {
        let id = format!("a1b2c3d4-e5f6-4789-abcd-00000000003{i}");
        make_observation(&cogz_dir, &id, &format!("Note {i}"), "content");
        let entity = make_db_observation(&id, &format!("Note {i}"), "content");
        insert_entity(&conn, &entity).unwrap();
        insert_edge(
            &conn,
            &Edge {
                source_id: id.clone(),
                target_id: "code-uuid-4".to_string(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }
    drop(conn);

    let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-4".to_string()]);
    assert_eq!(count, 2);
}

#[test]
fn flags_auto_references_edges() {
    // auto_references edges (created by auto_link) must also trigger
    // stale flagging when the target code entity changes. Without
    // this, auto-linked knowledge silently remains active.
    let (dir, storage) = setup();
    let cogz_dir = dir.path().join(".cogz");

    let conn = storage.conn();
    let code_entity = Entity::new("code-auto-1", "function", "auto_func", "fn auto_func() {}");
    insert_entity(&conn, &code_entity).unwrap();

    // Observation linked via auto_references (not manual references).
    make_observation(
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000023",
        "Note about auto_func",
        "content",
    );
    let obs_entity = make_db_observation(
        "a1b2c3d4-e5f6-4789-abcd-000000000023",
        "Note about auto_func",
        "content",
    );
    insert_entity(&conn, &obs_entity).unwrap();
    insert_edge(
        &conn,
        &Edge {
            source_id: "a1b2c3d4-e5f6-4789-abcd-000000000023".to_string(),
            target_id: "code-auto-1".to_string(),
            edge_type: "auto_references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    drop(conn);

    let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-auto-1".to_string()]);
    assert_eq!(count, 1);

    let conn = storage.conn();
    let entity = storage::crud::get_entity(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000023").unwrap();
    assert_eq!(entity.status, "stale");
}
