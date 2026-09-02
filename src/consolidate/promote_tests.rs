//! Tests for observation-to-rule promotion.

use super::*;
use crate::storage::crud::{Entity, insert_entity};
use crate::storage::edges::{Edge, insert_edge};
use crate::storage::ensure_vec_extension;
use rusqlite::Connection;

fn setup() -> crate::storage::Storage {
    ensure_vec_extension();
    crate::storage::Storage::open_memory().unwrap()
}

fn config(threshold: u32) -> ConsolidationConfig {
    ConsolidationConfig {
        dedup_threshold: 0.92,
        title_match_threshold: 0.85,
        contradiction_check: true,
        promotion_threshold: threshold,
        contradiction_threshold: 0.70,
        contradiction_cosine_threshold: 0.85,
        contradiction_length_ratio: 5.0,
        dedup_nli_threshold: 0.85,
    }
}

fn add_supports_edge(conn: &Connection, source: &str, target: &str) {
    insert_edge(
        conn,
        &Edge {
            source_id: source.to_string(),
            target_id: target.to_string(),
            edge_type: "supports".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
}

#[test]
fn dry_run_reports_candidates_without_changes() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new("obs-1", "observation", "Obs 1", "content"),
        )
        .unwrap();
        for i in 2..=4 {
            insert_entity(
                &conn,
                &Entity::new(&format!("sup-{i}"), "observation", &format!("Sup {i}"), "c"),
            )
            .unwrap();
            add_supports_edge(&conn, &format!("sup-{i}"), "obs-1");
        }
    }

    let results = run_promotion(&storage, &cogz_dir, &config(3), true).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].observation_id, "obs-1");
    assert!(results[0].new_rule_id.is_empty());

    // No rule entity should exist.
    let conn = storage.conn();
    let rule_count = crate::storage::crud::count_by_type(&conn, "rule").unwrap();
    assert_eq!(rule_count, 0);
}

#[test]
fn promotes_observation_with_enough_supporters() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new(
                "obs-1",
                "observation",
                "Test Obs",
                "Always use batched queries",
            ),
        )
        .unwrap();
        for i in 2..=4 {
            insert_entity(
                &conn,
                &Entity::new(&format!("sup-{i}"), "observation", &format!("Sup {i}"), "c"),
            )
            .unwrap();
            add_supports_edge(&conn, &format!("sup-{i}"), "obs-1");
        }
    }

    let results = run_promotion(&storage, &cogz_dir, &config(3), false).unwrap();
    assert_eq!(results.len(), 1);
    assert!(!results[0].new_rule_id.is_empty());

    let conn = storage.conn();
    let rule_count = crate::storage::crud::count_by_type(&conn, "rule").unwrap();
    assert_eq!(rule_count, 1);

    // Verify derived_from edge.
    let edges = crate::storage::edges::get_edges_from(&conn, &results[0].new_rule_id).unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e.edge_type == "derived_from" && e.target_id == "obs-1")
    );

    // Verify supports edges from supporting_ids frontmatter.
    // Without sync_references, these edges were missing — the rule file
    // had supporting_ids in frontmatter but no DB edges.
    let supports_edges: Vec<_> = edges.iter().filter(|e| e.edge_type == "supports").collect();
    assert_eq!(
        supports_edges.len(),
        3,
        "promoted rule should have supports edges to all 3 supporters"
    );
    for i in 2..=4 {
        assert!(
            supports_edges
                .iter()
                .any(|e| e.target_id == format!("sup-{i}")),
            "missing supports edge to sup-{i}"
        );
    }

    // Verify rule_promoted event.
    let events =
        crate::storage::events::get_events_for_entity(&conn, &results[0].new_rule_id).unwrap();
    assert!(events.iter().any(|e| e.event_type == "rule_promoted"));
}

#[test]
fn no_promotion_below_threshold() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    {
        let conn = storage.conn();
        insert_entity(&conn, &Entity::new("obs-1", "observation", "Obs 1", "c")).unwrap();
        insert_entity(&conn, &Entity::new("sup-1", "observation", "Sup 1", "c")).unwrap();
        add_supports_edge(&conn, "sup-1", "obs-1");
    }

    let results = run_promotion(&storage, &cogz_dir, &config(3), false).unwrap();
    assert!(results.is_empty());
}

#[test]
fn no_duplicate_promotion_after_first() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new("obs-1", "observation", "Obs 1", "content"),
        )
        .unwrap();
        for i in 2..=4 {
            insert_entity(
                &conn,
                &Entity::new(&format!("sup-{i}"), "observation", &format!("Sup {i}"), "c"),
            )
            .unwrap();
            add_supports_edge(&conn, &format!("sup-{i}"), "obs-1");
        }
    }

    // First promotion should succeed.
    let results = run_promotion(&storage, &cogz_dir, &config(3), false).unwrap();
    assert_eq!(results.len(), 1);

    // Second promotion should not re-promote the same observation.
    let results2 = run_promotion(&storage, &cogz_dir, &config(3), false).unwrap();
    assert!(
        results2.is_empty(),
        "observation should not be promoted twice"
    );

    let conn = storage.conn();
    let rule_count = crate::storage::crud::count_by_type(&conn, "rule").unwrap();
    assert_eq!(rule_count, 1);
}
