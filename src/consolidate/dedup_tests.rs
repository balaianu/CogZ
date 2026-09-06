//! Tests for duplicate detection.

use super::*;
use crate::storage::crud::{Entity, insert_entity};
use crate::storage::ensure_vec_extension;
use crate::storage::schema::run_migrations;

fn setup() -> Connection {
    ensure_vec_extension();
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, 768).unwrap();
    conn
}

fn config() -> ConsolidationConfig {
    ConsolidationConfig {
        dedup_threshold: 0.85,
        title_match_threshold: 0.85,
        contradiction_check: true,
        promotion_threshold: 3,
        contradiction_threshold: 0.70,
        contradiction_cosine_threshold: 0.85,
        contradiction_length_ratio: 5.0,
        dedup_nli_threshold: 0.85,
    }
}

#[test]
fn exact_title_match_detected() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "FTS5 ranking bug", "content"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("u2", "observation", "FTS5 ranking bug", "content"),
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "u2",
        "FTS5 ranking bug",
        "observation",
        None,
        &config(),
    );
    let warning = result.duplicate_warning.expect("should detect duplicate");
    assert_eq!(warning.existing_id, "u1");
    assert_eq!(warning.title_match, "exact");
    assert_eq!(warning.similarity, 1.0);
}

#[test]
fn case_insensitive_title_match() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "knowledge", "Search Architecture", "c"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("u2", "knowledge", "search architecture", "c"),
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "u2",
        "search architecture",
        "knowledge",
        None,
        &config(),
    );
    assert!(result.duplicate_warning.is_some());
    assert_eq!(result.duplicate_warning.unwrap().title_match, "exact");
}

#[test]
fn fuzzy_title_match_detected() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "rule", "Use parameterized queries", "c"),
    )
    .unwrap();

    // Different punctuation/case, same normalized form
    let result = check_duplicate(
        &conn,
        "u2",
        "use-parameterized queries!",
        "rule",
        None,
        &config(),
    );
    assert!(result.duplicate_warning.is_some());
    assert_eq!(result.duplicate_warning.unwrap().title_match, "fuzzy");
}

#[test]
fn no_duplicate_when_titles_differ() {
    let conn = setup();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "Bug in search", "c"),
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "u2",
        "Feature request: dark mode",
        "observation",
        None,
        &config(),
    );
    assert!(result.duplicate_warning.is_none());
    assert!(!result.dedup_flagged);
}

#[test]
fn skips_self_in_title_match() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "Same title", "c")).unwrap();

    let result = check_duplicate(&conn, "u1", "Same title", "observation", None, &config());
    assert!(result.duplicate_warning.is_none());
}

#[test]
fn embedding_similarity_flagged() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

    // Identical embeddings → distance ~0 → similarity ~1.0
    let embedding = vec![0.1_f32; 768];
    crate::storage::embeddings::insert_embedding(&conn, "u1", "observation", &embedding).unwrap();

    let result = check_duplicate(&conn, "u2", "B", "observation", Some(&embedding), &config());
    assert!(result.dedup_flagged);
    assert!(result.duplicate_warning.is_some());
}

#[test]
fn no_embedding_check_without_embedding() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();

    let result = check_duplicate(
        &conn,
        "u2",
        "Different title",
        "observation",
        None,
        &config(),
    );
    assert!(!result.dedup_flagged);
    assert!(result.duplicate_warning.is_none());
}

#[test]
fn normalize_title_strips_punctuation() {
    assert_eq!(
        normalize_title("Use-Parameterized Queries!"),
        "use parameterized queries"
    );
    assert_eq!(normalize_title("FTS5: Ranking Bug"), "fts5 ranking bug");
    assert_eq!(normalize_title("  multiple   spaces  "), "multiple spaces");
}

#[test]
fn nli_confirms_identical_texts_as_duplicate() {
    use crate::embed::MockNliModel;
    assert!(confirm_duplicate_nli(
        Some(&MockNliModel),
        "The bug is in search",
        "The bug is in search",
        0.85,
    ));
}

#[test]
fn nli_rejects_unrelated_texts_as_duplicate() {
    use crate::embed::MockNliModel;
    assert!(!confirm_duplicate_nli(
        Some(&MockNliModel),
        "The bug is in search",
        "The config file is missing",
        0.85,
    ));
}

#[test]
fn nli_rejects_contradiction_as_duplicate() {
    use crate::embed::MockNliModel;
    assert!(!confirm_duplicate_nli(
        Some(&MockNliModel),
        "The bug is in search",
        "The bug is not in search",
        0.85,
    ));
}

#[test]
fn nli_dedup_without_model_returns_false() {
    assert!(!confirm_duplicate_nli(None, "same text", "same text", 0.85,));
}
