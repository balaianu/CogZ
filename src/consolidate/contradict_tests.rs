//! Tests for contradiction detection.

use super::*;
use crate::embed::MockNliModel;
use crate::storage::crud::{Entity, insert_entity};
use crate::storage::ensure_vec_extension;
use crate::storage::schema::run_migrations;

fn setup() -> Connection {
    ensure_vec_extension();
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, 768).unwrap();
    conn
}

fn config(check: bool) -> ConsolidationConfig {
    ConsolidationConfig {
        dedup_threshold: 0.85,
        title_match_threshold: 0.85,
        contradiction_check: check,
        promotion_threshold: 3,
        contradiction_threshold: 0.70,
        contradiction_cosine_threshold: 0.85,
        contradiction_length_ratio: 5.0,
        dedup_nli_threshold: 0.85,
    }
}

#[test]
fn disabled_in_config_returns_empty() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "yes")).unwrap();
    let result = check_contradiction(
        &conn,
        "u2",
        "no",
        "observation",
        Some(&MockNliModel),
        None,
        &config(false),
    );
    assert!(!result.contradiction_flagged);
    assert!(result.contradicts_ids.is_empty());
}

#[test]
fn no_model_returns_empty() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "yes")).unwrap();
    let result = check_contradiction(&conn, "u2", "no", "observation", None, None, &config(true));
    assert!(!result.contradiction_flagged);
}

#[test]
fn detects_contradiction_with_mock_model() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "A", "The bug is in search"),
    )
    .unwrap();

    let result = check_contradiction(
        &conn,
        "u2",
        "The bug is not in search",
        "observation",
        Some(&MockNliModel),
        None,
        &config(true),
    );
    assert!(result.contradiction_flagged);
    assert_eq!(result.contradicts_ids, vec!["u1".to_string()]);
}

#[test]
fn no_contradiction_for_entailment() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "A", "The bug is in search"),
    )
    .unwrap();

    let result = check_contradiction(
        &conn,
        "u2",
        "The bug is in search",
        "observation",
        Some(&MockNliModel),
        None,
        &config(true),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn skips_self() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "A", "The bug is in search"),
    )
    .unwrap();

    let result = check_contradiction(
        &conn,
        "u1",
        "The bug is in search",
        "observation",
        Some(&MockNliModel),
        None,
        &config(true),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn length_ratio_filter_skips_uneven_pairs() {
    let conn = setup();
    let long_text = "This is a very long observation that goes on and on \
                     about many different topics in great detail, covering \
                     architecture, design patterns, testing strategies, and \
                     deployment considerations for the project.";
    insert_entity(&conn, &Entity::new("u1", "observation", "A", long_text)).unwrap();

    // "not" in a 4-word text vs a 50+ word text — length ratio > 5:1.
    let result = check_contradiction(
        &conn,
        "u2",
        "This is not relevant",
        "observation",
        Some(&MockNliModel),
        None,
        &config(true),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn identical_text_fast_path_skips_nli() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "A", "The bug is in search"),
    )
    .unwrap();

    // Same text — should be entailment, not contradiction.
    let result = check_contradiction(
        &conn,
        "u2",
        "The bug is in search",
        "observation",
        Some(&MockNliModel),
        None,
        &config(true),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn low_contradiction_probability_not_flagged() {
    let conn = setup();
    // Two unrelated texts — mock returns neutral (0.85 neutral, 0.05 contra).
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "A", "The config file is missing"),
    )
    .unwrap();

    let result = check_contradiction(
        &conn,
        "u2",
        "The bug is in search",
        "observation",
        Some(&MockNliModel),
        None,
        &config(true),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn record_contradictions_creates_edges_and_event() {
    use crate::files::frontmatter::{FmValue, Frontmatter, serialize as serialize_fm};
    use crate::storage::Storage;

    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(&cogz_dir).unwrap();

    let storage = Storage::open(&cogz_dir.join("test.db"), 768).unwrap();
    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000031",
                "observation",
                "A",
                "c",
            ),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000032",
                "observation",
                "B",
                "c",
            ),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000033",
                "observation",
                "C",
                "c",
            ),
        )
        .unwrap();
    }

    // Write a file for u3 so record_contradictions can update it.
    let file_path = cogz_dir.join("u3.md");
    let mut fm = Frontmatter::new();
    fm.insert(
        "id",
        FmValue::String("a1b2c3d4-e5f6-4789-abcd-000000000033".to_string()),
    );
    fm.insert("title", FmValue::String("C".to_string()));
    fm.insert("type", FmValue::String("observation".to_string()));
    fm.insert("status", FmValue::String("active".to_string()));
    fm.insert(
        "created_at",
        FmValue::String("2026-01-01T00:00:00Z".to_string()),
    );
    fm.insert(
        "updated_at",
        FmValue::String("2026-01-01T00:00:00Z".to_string()),
    );
    fm.insert("references", FmValue::Array(vec![]));
    std::fs::write(&file_path, format!("---\n{}---\n\nc", serialize_fm(&fm))).unwrap();

    record_contradictions(
        &storage,
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000033",
        &[
            "a1b2c3d4-e5f6-4789-abcd-000000000031".to_string(),
            "a1b2c3d4-e5f6-4789-abcd-000000000032".to_string(),
        ],
        &file_path,
    )
    .unwrap();

    let conn = storage.conn();
    let edges =
        crate::storage::edges::get_edges_from(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000033")
            .unwrap();
    let contradict_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.edge_type == "contradicts")
        .collect();
    assert_eq!(contradict_edges.len(), 2);

    let events = crate::storage::events::get_events_for_entity(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000033",
    )
    .unwrap();
    assert!(events.iter().any(|e| e.event_type == "contradiction_found"));

    // Verify the file was updated with contradicts frontmatter.
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("contradicts"));
}
