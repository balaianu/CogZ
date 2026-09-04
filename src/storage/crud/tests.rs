//! Tests for the crud module.

use super::super::edges;
use super::super::ensure_vec_extension;
use super::super::events;
use super::super::schema::run_migrations;
use super::*;

use rusqlite::Connection;

fn setup() -> Connection {
    ensure_vec_extension();
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, 768).unwrap();
    conn
}

#[test]
fn insert_and_get_entity() {
    let conn = setup();
    let entity = Entity::new("uuid-1", "observation", "Test", "Content here");
    insert_entity(&conn, &entity).unwrap();

    let fetched = get_entity(&conn, "uuid-1").unwrap();
    assert_eq!(fetched.id, "uuid-1");
    assert_eq!(fetched.r#type, "observation");
    assert_eq!(fetched.title, Some("Test".to_string()));
    assert_eq!(fetched.content, "Content here");
    assert_eq!(fetched.status, "active");
}

#[test]
fn get_nonexistent_entity_errors() {
    let conn = setup();
    let result = get_entity(&conn, "nonexistent");
    assert!(matches!(result, Err(StorageError::EntityNotFound(_))));
}

#[test]
fn update_entity_content() {
    let conn = setup();
    let mut entity = Entity::new("uuid-1", "knowledge", "Title", "Old content");
    insert_entity(&conn, &entity).unwrap();

    entity.content = "New content".to_string();
    update_entity(&conn, &entity).unwrap();

    let fetched = get_entity(&conn, "uuid-1").unwrap();
    assert_eq!(fetched.content, "New content");
}

#[test]
fn update_status_legal_transition() {
    let conn = setup();
    let entity = Entity::new("uuid-1", "observation", "Test", "Content");
    insert_entity(&conn, &entity).unwrap();

    update_status(&conn, "uuid-1", "stale").unwrap();
    let fetched = get_entity(&conn, "uuid-1").unwrap();
    assert_eq!(fetched.status, "stale");
}

#[test]
fn update_status_illegal_transition() {
    let conn = setup();
    let entity = Entity::new("uuid-1", "observation", "Test", "Content");
    insert_entity(&conn, &entity).unwrap();

    update_status(&conn, "uuid-1", "rejected").unwrap();

    let result = update_status(&conn, "uuid-1", "active");
    assert!(matches!(
        result,
        Err(StorageError::IllegalTransition { .. })
    ));
}

#[test]
fn delete_entity_removes_from_fts() {
    let conn = setup();
    let entity = Entity::new("uuid-1", "observation", "Searchable", "unique content");
    insert_entity(&conn, &entity).unwrap();

    let fts_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 1);

    delete_entity(&conn, "uuid-1").unwrap();

    let fts_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 0);
}

#[test]
fn tombstone_entity_clears_content_and_sets_status() {
    let conn = setup();
    let entity = Entity::new("uuid-1", "observation", "Title", "content");
    insert_entity(&conn, &entity).unwrap();
    // Pruning requires rejected or superseded status.
    update_status(&conn, "uuid-1", "rejected").unwrap();

    tombstone_entity(&conn, "uuid-1").unwrap();

    let fetched = get_entity(&conn, "uuid-1").unwrap();
    assert_eq!(fetched.status, "pruned");
    assert_eq!(fetched.content, "");
    assert!(fetched.title.is_none());
    assert!(fetched.content_hash.is_none());
}

#[test]
fn tombstone_entity_rejects_active_status() {
    let conn = setup();
    let entity = Entity::new("uuid-active", "observation", "Title", "content");
    insert_entity(&conn, &entity).unwrap();

    let result = tombstone_entity(&conn, "uuid-active");
    assert!(result.is_err(), "tombstoning an active entity should fail");
}

#[test]
fn tombstone_entity_nonexistent_errors() {
    let conn = setup();
    let result = tombstone_entity(&conn, "nonexistent");
    assert!(matches!(result, Err(StorageError::EntityNotFound(_))));
}

#[test]
fn delete_entity_cascade_removes_edges_and_nullifies_events() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("a", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("b", "observation", "B", "c")).unwrap();

    edges::insert_edge(
        &conn,
        &edges::Edge {
            source_id: "a".to_string(),
            target_id: "b".to_string(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();

    events::record_event(
        &conn,
        events::EventType::ObservationCreated,
        Some("a"),
        &serde_json::json!({}),
    )
    .unwrap();

    delete_entity_cascade(&conn, "a").unwrap();

    assert!(get_entity(&conn, "a").is_err());

    let edges = edges::get_edges_from(&conn, "a").unwrap();
    assert!(edges.is_empty());

    let event_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE entity_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(event_count, 1);
}

#[test]
fn count_entities_by_type() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u3", "rule", "R", "c")).unwrap();

    assert_eq!(count_by_type(&conn, "observation").unwrap(), 2);
    assert_eq!(count_by_type(&conn, "rule").unwrap(), 1);
    assert_eq!(count_by_type(&conn, "knowledge").unwrap(), 0);
}

#[test]
fn entity_type_roundtrip() {
    assert_eq!(
        EntityType::parse("observation").unwrap(),
        EntityType::Observation
    );
    assert_eq!(EntityType::Observation.as_str(), "observation");
    assert!(EntityType::from_str("invalid").is_err());
}

#[test]
fn is_code_distinguishes_entity_kinds() {
    assert!(!EntityType::Observation.is_code());
    assert!(!EntityType::Rule.is_code());
    assert!(!EntityType::Knowledge.is_code());
    assert!(EntityType::Function.is_code());
    assert!(EntityType::Class.is_code());
    assert!(EntityType::File.is_code());
    assert!(EntityType::Module.is_code());
}

#[test]
fn mark_entities_stale_by_ids_marks_active_only() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("e1", "function", "foo", "c")).unwrap();
    insert_entity(&conn, &Entity::new("e2", "function", "bar", "c")).unwrap();
    let mut stale = Entity::new("e3", "function", "baz", "c");
    stale.status = "stale".to_string();
    insert_entity(&conn, &stale).unwrap();

    let count = mark_entities_stale_by_ids(
        &conn,
        &["e1".to_string(), "e2".to_string(), "e3".to_string()],
    )
    .unwrap();

    assert_eq!(count, 2, "only active entities should be marked stale");
    assert_eq!(get_entity(&conn, "e1").unwrap().status, "stale");
    assert_eq!(get_entity(&conn, "e2").unwrap().status, "stale");
    assert_eq!(get_entity(&conn, "e3").unwrap().status, "stale");
}

#[test]
fn mark_entities_stale_by_ids_empty_input() {
    let conn = setup();
    let count = mark_entities_stale_by_ids(&conn, &[]).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn mark_entities_stale_by_ids_nonexistent_ids() {
    let conn = setup();
    let count = mark_entities_stale_by_ids(&conn, &["nonexistent".to_string()]).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn delete_entities_cascade_batch_removes_multiple() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("b1", "observation", "B1", "c")).unwrap();
    insert_entity(&conn, &Entity::new("b2", "observation", "B2", "c")).unwrap();
    insert_entity(&conn, &Entity::new("b3", "observation", "B3", "c")).unwrap();

    let removed = delete_entities_cascade_batch(
        &conn,
        &[
            "b1".to_string(),
            "b2".to_string(),
            "nonexistent".to_string(),
        ],
    )
    .unwrap();

    assert_eq!(removed, 2);
    assert!(get_entity(&conn, "b1").is_err());
    assert!(get_entity(&conn, "b2").is_err());
    assert!(get_entity(&conn, "b3").is_ok());
}

#[test]
fn delete_entities_cascade_batch_empty_input() {
    let conn = setup();
    let removed = delete_entities_cascade_batch(&conn, &[]).unwrap();
    assert_eq!(removed, 0);
}
