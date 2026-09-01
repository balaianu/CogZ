//! Tests for the query module.

use super::super::crud::{Entity, insert_entity};
use super::super::ensure_vec_extension;
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
fn query_entities_by_type() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "content a")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "rule", "B", "content b")).unwrap();

    let result = get_entities_by_type(&conn, "observation", Some("active"), 20).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "u1");
}

#[test]
fn query_entities_by_status() {
    let conn = setup();
    let mut e1 = Entity::new("u1", "observation", "A", "c");
    e1.status = "stale".to_string();
    insert_entity(&conn, &e1).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

    let active = get_entities_by_type(&conn, "observation", Some("active"), 20).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, "u2");

    let stale = get_entities_by_type(&conn, "observation", Some("stale"), 20).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, "u1");
}

#[test]
fn fts_search_finds_content() {
    let conn = setup();
    let e1 = Entity::new(
        "u1",
        "observation",
        "FTS5 ranking bug",
        "The RRF fusion produces incorrect rankings",
    );
    insert_entity(&conn, &e1).unwrap();
    insert_entity(
        &conn,
        &Entity::new("u2", "rule", "Unrelated", "completely different content"),
    )
    .unwrap();

    let results = fts_search(&conn, "ranking", None, Some("active"), 20).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "u1");
}

#[test]
fn fts_search_with_type_filter() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "ranking", "ranking content"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("u2", "rule", "ranking", "ranking content"),
    )
    .unwrap();

    let obs = fts_search(&conn, "ranking", Some("observation"), Some("active"), 20).unwrap();
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].r#type, "observation");
}

#[test]
fn fts_search_with_hyphenated_query() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new(
            "u1",
            "knowledge",
            "tree-sitter parsing",
            "AST extraction with tree-sitter",
        ),
    )
    .unwrap();

    // Hyphenated query should not crash (FTS5 treats - as NOT)
    let results = fts_search(&conn, "tree-sitter", None, Some("active"), 20).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "u1");
}

#[test]
fn fts_search_multi_word_query() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new(
            "u1",
            "knowledge",
            "Search System Architecture",
            "The search system uses hybrid FTS5 and vector search",
        ),
    )
    .unwrap();

    // Multi-word query should match all terms in any position (implicit AND),
    // not require them to be adjacent (phrase matching).
    let results = fts_search(&conn, "search architecture", None, Some("active"), 20).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "u1");

    // Three-word query
    let results = fts_search(
        &conn,
        "search system architecture",
        None,
        Some("active"),
        20,
    )
    .unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn count_stale_entities() {
    let conn = setup();
    let mut e1 = Entity::new("u1", "observation", "A", "c");
    e1.status = "stale".to_string();
    insert_entity(&conn, &e1).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

    assert_eq!(count_stale(&conn).unwrap(), 1);
}

#[test]
fn get_entity_counts_by_type() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u3", "rule", "R", "c")).unwrap();

    let counts = entity_counts_by_type(&conn).unwrap();
    let obs = counts.iter().find(|(t, _)| t == "observation").unwrap();
    assert_eq!(obs.1, 2);
}

#[test]
fn rules_sorted_by_confidence_then_recency() {
    let conn = setup();

    let mut r1 = Entity::new("r1", "rule", "Low confidence", "c");
    r1.properties = serde_json::json!({"confidence": 0.3});
    insert_entity(&conn, &r1).unwrap();

    let mut r2 = Entity::new("r2", "rule", "High confidence", "c");
    r2.properties = serde_json::json!({"confidence": 0.9});
    insert_entity(&conn, &r2).unwrap();

    let r3 = Entity::new("r3", "rule", "Default confidence", "c");
    insert_entity(&conn, &r3).unwrap();

    let rules = get_rules_by_confidence(&conn, Some("active"), 20).unwrap();
    assert_eq!(rules[0].id, "r3");
    assert_eq!(rules[1].id, "r2");
    assert_eq!(rules[2].id, "r1");
}

#[test]
fn find_prune_candidates_returns_terminal_observations_with_file_path() {
    let conn = setup();

    let mut rejected = Entity::new("u1", "observation", "Rejected", "c");
    rejected.status = "rejected".to_string();
    rejected.file_path = Some("observations/u1.md".to_string());
    insert_entity(&conn, &rejected).unwrap();

    let mut superseded = Entity::new("u2", "observation", "Superseded", "c");
    superseded.status = "superseded".to_string();
    superseded.file_path = Some("observations/u2.md".to_string());
    insert_entity(&conn, &superseded).unwrap();

    let mut active = Entity::new("u3", "observation", "Active", "c");
    active.file_path = Some("observations/u3.md".to_string());
    insert_entity(&conn, &active).unwrap();

    let mut no_file = Entity::new("u4", "observation", "No file", "c");
    no_file.status = "rejected".to_string();
    insert_entity(&conn, &no_file).unwrap();

    let candidates = find_prune_candidates(&conn).unwrap();
    assert_eq!(candidates.len(), 2);
    let ids: Vec<&str> = candidates.iter().map(|c| c.entity_id.as_str()).collect();
    assert!(ids.contains(&"u1"));
    assert!(ids.contains(&"u2"));
}

#[test]
fn get_oldest_tombstone_ids_returns_oldest_first() {
    let conn = setup();

    let mut t1 = Entity::new("t1", "observation", "Old", "c");
    t1.status = "pruned".to_string();
    t1.updated_at = "2026-01-01T00:00:00Z".to_string();
    insert_entity(&conn, &t1).unwrap();

    let mut t2 = Entity::new("t2", "observation", "Newer", "c");
    t2.status = "pruned".to_string();
    t2.updated_at = "2026-06-01T00:00:00Z".to_string();
    insert_entity(&conn, &t2).unwrap();

    let ids = get_oldest_tombstone_ids(&conn, 1).unwrap();
    assert_eq!(ids, vec!["t1".to_string()]);

    let ids = get_oldest_tombstone_ids(&conn, 2).unwrap();
    assert_eq!(ids, vec!["t1".to_string(), "t2".to_string()]);
}
