//! Integration tests for Phase 9 consolidation.
//!
//! Tests cover:
//! - Dedup via the consolidate module (title + embedding similarity)
//! - Contradiction detection with MockNliModel
//! - Observation-to-rule promotion (threshold-based)
//! - Duplicate merge with edge redirection and superseded marking
//! - Graceful degradation when NLI model is unavailable
//! - `cogz consolidate` dry-run behavior

use cogz::config::ConsolidationConfig;
use cogz::consolidate::contradict::{check_contradiction, record_contradictions};
use cogz::consolidate::dedup::check_duplicate;
use cogz::consolidate::merge::run_merge;
use cogz::consolidate::promote::run_promotion;
use cogz::embed::MockNliModel;
use cogz::files::frontmatter::{FmValue, Frontmatter, serialize as serialize_fm};
use cogz::storage::Storage;
use cogz::storage::crud::{Entity, insert_entity};
use cogz::storage::edges::{Edge, get_edges_from, get_edges_to, insert_edge};
use cogz::storage::embeddings::insert_embedding;

fn setup() -> Storage {
    Storage::open_memory().unwrap()
}

fn config() -> ConsolidationConfig {
    ConsolidationConfig {
        dedup_threshold: 0.92,
        title_match_threshold: 0.85,
        contradiction_check: true,
        promotion_threshold: 3,
    }
}

fn write_observation_file(cogz_dir: &std::path::Path, id: &str, title: &str, created: &str) {
    let dir = cogz_dir.join("observations").join("2026-01");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.md"));
    let mut fm = Frontmatter::new();
    fm.insert("id", FmValue::String(id.to_string()));
    fm.insert("title", FmValue::String(title.to_string()));
    fm.insert("type", FmValue::String("observation".to_string()));
    fm.insert("status", FmValue::String("active".to_string()));
    fm.insert("created_at", FmValue::String(created.to_string()));
    fm.insert("updated_at", FmValue::String(created.to_string()));
    fm.insert("references", FmValue::Array(vec![]));
    std::fs::write(&path, format!("---\n{}---\n\ncontent", serialize_fm(&fm))).unwrap();
}

// ─── Dedup integration ───────────────────────────────────────────

#[test]
fn dedup_exact_title_match_via_consolidate_module() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new("u1", "observation", "FTS5 ranking bug", "c"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("u2", "observation", "FTS5 ranking bug", "c"),
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
    assert!(result.duplicate_warning.is_some());
    assert_eq!(result.duplicate_warning.unwrap().title_match, "exact");
}

#[test]
fn dedup_embedding_similarity_flagged() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

    let embedding = vec![0.1_f32; 768];
    insert_embedding(&conn, "u1", &embedding).unwrap();

    let result = check_duplicate(&conn, "u2", "B", "observation", Some(&embedding), &config());
    assert!(result.dedup_flagged);
}

#[test]
fn dedup_no_false_positive_for_different_titles() {
    let storage = setup();
    let conn = storage.conn();
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

// ─── Contradiction integration ───────────────────────────────────

#[test]
fn contradiction_detected_with_mock_nli() {
    let storage = setup();
    let conn = storage.conn();
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
        &config(),
    );
    assert!(result.contradiction_flagged);
    assert_eq!(result.contradicts_ids, vec!["u1".to_string()]);
}

#[test]
fn contradiction_skipped_when_nli_unavailable() {
    let storage = setup();
    let conn = storage.conn();
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
        None,
        &config(),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn contradiction_records_edges_and_event() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

    record_contradictions(&conn, "u2", &["u1".to_string()]).unwrap();

    let edges = get_edges_from(&conn, "u2").unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e.edge_type == "contradicts" && e.target_id == "u1")
    );
}

// ─── Promotion integration ───────────────────────────────────────

#[test]
fn promotion_creates_rule_file_and_derived_from_edge() {
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
            insert_edge(
                &conn,
                &Edge {
                    source_id: format!("sup-{i}"),
                    target_id: "obs-1".to_string(),
                    edge_type: "supports".to_string(),
                    weight: 1.0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .unwrap();
        }
    }

    let results = run_promotion(&storage, &cogz_dir, &config(), false).unwrap();
    assert_eq!(results.len(), 1);

    let conn = storage.conn();
    let rule_count = cogz::storage::crud::count_by_type(&conn, "rule").unwrap();
    assert_eq!(rule_count, 1);

    let edges = get_edges_from(&conn, &results[0].new_rule_id).unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e.edge_type == "derived_from" && e.target_id == "obs-1")
    );
}

#[test]
fn promotion_dry_run_makes_no_changes() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    {
        let conn = storage.conn();
        insert_entity(&conn, &Entity::new("obs-1", "observation", "Obs 1", "c")).unwrap();
        for i in 2..=4 {
            insert_entity(
                &conn,
                &Entity::new(&format!("sup-{i}"), "observation", &format!("Sup {i}"), "c"),
            )
            .unwrap();
            insert_edge(
                &conn,
                &Edge {
                    source_id: format!("sup-{i}"),
                    target_id: "obs-1".to_string(),
                    edge_type: "supports".to_string(),
                    weight: 1.0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .unwrap();
        }
    }

    let results = run_promotion(&storage, &cogz_dir, &config(), true).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].new_rule_id.is_empty());

    let conn = storage.conn();
    assert_eq!(
        cogz::storage::crud::count_by_type(&conn, "rule").unwrap(),
        0
    );
}

// ─── Merge integration ───────────────────────────────────────────

#[test]
fn merge_supersedes_duplicate_and_redirects_edges() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    let obs_dir = cogz_dir.join("observations").join("2026-01");
    std::fs::create_dir_all(&obs_dir).unwrap();

    let embedding = vec![0.1_f32; 768];
    {
        let conn = storage.conn();
        let mut e1 = Entity::new("obs-1", "observation", "A", "content");
        e1.created_at = "2026-01-01T00:00:00Z".to_string();
        e1.file_path = Some(obs_dir.join("obs-1.md").to_string_lossy().to_string());
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(&conn, "obs-1", &embedding).unwrap();

        let mut e2 = Entity::new("obs-2", "observation", "B", "content");
        e2.created_at = "2026-02-01T00:00:00Z".to_string();
        e2.file_path = Some(obs_dir.join("obs-2.md").to_string_lossy().to_string());
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(&conn, "obs-2", &embedding).unwrap();

        insert_entity(&conn, &Entity::new("kn-1", "knowledge", "K", "c")).unwrap();
        insert_edge(
            &conn,
            &Edge {
                source_id: "kn-1".to_string(),
                target_id: "obs-2".to_string(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    write_observation_file(&cogz_dir, "obs-1", "A", "2026-01-01T00:00:00Z");
    write_observation_file(&cogz_dir, "obs-2", "B", "2026-02-01T00:00:00Z");

    let results = run_merge(&storage, &cogz_dir, &config(), false).unwrap();
    assert_eq!(results.len(), 1);

    let conn = storage.conn();
    let e2 = cogz::storage::crud::get_entity(&conn, "obs-2").unwrap();
    assert_eq!(e2.status, "superseded");

    let edges = get_edges_to(&conn, "obs-1").unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e.source_id == "kn-1" && e.edge_type == "references")
    );

    let old_edges = get_edges_to(&conn, "obs-2").unwrap();
    assert!(old_edges.is_empty());
}

#[test]
fn merge_dry_run_makes_no_changes() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");

    let embedding = vec![0.1_f32; 768];
    {
        let conn = storage.conn();
        let mut e1 = Entity::new("obs-1", "observation", "A", "content");
        e1.created_at = "2026-01-01T00:00:00Z".to_string();
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(&conn, "obs-1", &embedding).unwrap();

        let mut e2 = Entity::new("obs-2", "observation", "B", "content");
        e2.created_at = "2026-02-01T00:00:00Z".to_string();
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(&conn, "obs-2", &embedding).unwrap();
    }

    let results = run_merge(&storage, &cogz_dir, &config(), true).unwrap();
    assert_eq!(results.len(), 1);

    let conn = storage.conn();
    let e2 = cogz::storage::crud::get_entity(&conn, "obs-2").unwrap();
    assert_eq!(e2.status, "active");
}

// ─── Graceful degradation ────────────────────────────────────────

#[test]
fn full_consolidation_works_without_nli_model() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    // No NLI model — promotion and merge should still work.
    let promoted = run_promotion(&storage, &cogz_dir, &config(), false).unwrap();
    let merged = run_merge(&storage, &cogz_dir, &config(), false).unwrap();
    assert!(promoted.is_empty());
    assert!(merged.is_empty());
}

#[test]
fn dedup_works_without_embeddings() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(&conn, &Entity::new("u1", "observation", "Same title", "c")).unwrap();

    let result = check_duplicate(&conn, "u2", "Same title", "observation", None, &config());
    assert!(result.duplicate_warning.is_some());
    assert!(!result.dedup_flagged);
}
