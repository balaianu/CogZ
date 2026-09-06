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
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000051",
            "observation",
            "FTS5 ranking bug",
            "c",
        ),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000052",
            "observation",
            "FTS5 ranking bug",
            "c",
        ),
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
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
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000051",
            "observation",
            "A",
            "c",
        ),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000052",
            "observation",
            "B",
            "c",
        ),
    )
    .unwrap();

    let embedding = vec![0.1_f32; 768];
    insert_embedding(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000051",
        "observation",
        &embedding,
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        "B",
        "observation",
        Some(&embedding),
        &config(),
    );
    assert!(result.dedup_flagged);
}

#[test]
fn dedup_no_false_positive_for_different_titles() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000051",
            "observation",
            "Bug in search",
            "c",
        ),
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        "Feature request: dark mode",
        "observation",
        None,
        &config(),
    );
    assert!(result.duplicate_warning.is_none());
    assert!(!result.dedup_flagged);
}

#[test]
fn dedup_ignores_non_active_entities() {
    // A rejected entity with the same title should not trigger a
    // duplicate warning for a new entity.
    let storage = setup();
    let conn = storage.conn();
    let mut rejected = Entity::new(
        "a1b2c3d4-e5f6-4789-abcd-000000000051",
        "observation",
        "Same title",
        "c",
    );
    rejected.status = "rejected".to_string();
    insert_entity(&conn, &rejected).unwrap();

    let result = check_duplicate(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        "Same title",
        "observation",
        None,
        &config(),
    );
    assert!(
        result.duplicate_warning.is_none(),
        "rejected entities should not trigger dedup"
    );
    assert!(!result.dedup_flagged);
}

// ─── Contradiction integration ───────────────────────────────────

#[test]
fn contradiction_detected_with_mock_nli() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000051",
            "observation",
            "A",
            "The bug is in search",
        ),
    )
    .unwrap();

    let result = check_contradiction(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        "The bug is not in search",
        "observation",
        Some(&MockNliModel),
        None,
        &config(),
    );
    assert!(result.contradiction_flagged);
    assert_eq!(
        result.contradicts_ids,
        vec!["a1b2c3d4-e5f6-4789-abcd-000000000051".to_string()]
    );
}

#[test]
fn contradiction_skipped_when_nli_unavailable() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000051",
            "observation",
            "A",
            "The bug is in search",
        ),
    )
    .unwrap();

    let result = check_contradiction(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        "The bug is not in search",
        "observation",
        None,
        None,
        &config(),
    );
    assert!(!result.contradiction_flagged);
}

#[test]
fn contradiction_records_edges_and_event() {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(&cogz_dir).unwrap();
    let storage = Storage::open(&cogz_dir.join("test.db"), 768).unwrap();
    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000051",
                "observation",
                "A",
                "c",
            ),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000052",
                "observation",
                "B",
                "c",
            ),
        )
        .unwrap();
    }

    // Write a file for u2 so record_contradictions can update it.
    let file_path = cogz_dir.join("u2.md");
    let mut fm = Frontmatter::new();
    fm.insert(
        "id",
        FmValue::String("a1b2c3d4-e5f6-4789-abcd-000000000052".to_string()),
    );
    fm.insert("title", FmValue::String("B".to_string()));
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
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        &["a1b2c3d4-e5f6-4789-abcd-000000000051".to_string()],
        &file_path,
    )
    .unwrap();

    let conn = storage.conn();
    let edges = get_edges_from(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000052").unwrap();
    assert!(
        edges.iter().any(|e| e.edge_type == "contradicts"
            && e.target_id == "a1b2c3d4-e5f6-4789-abcd-000000000051")
    );

    // Verify the file was updated with contradicts frontmatter.
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("contradicts"));
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
                "a1b2c3d4-e5f6-4789-abcd-000000000061",
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
                    target_id: "a1b2c3d4-e5f6-4789-abcd-000000000061".to_string(),
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
        edges.iter().any(|e| e.edge_type == "derived_from"
            && e.target_id == "a1b2c3d4-e5f6-4789-abcd-000000000061")
    );

    // Verify the rule file contains derived_from in frontmatter (file-backed edge).
    let rule_entity = cogz::storage::crud::get_entity(&conn, &results[0].new_rule_id).unwrap();
    let file_path = rule_entity.file_path.as_ref().unwrap();
    // Paths are stored relative to .cogz (e.g. "rules/foo.md").
    let abs_path = if file_path.starts_with(".cogz") {
        cogz_dir.parent().unwrap_or(&cogz_dir).join(file_path)
    } else {
        cogz_dir.join(file_path)
    };
    let file_content = std::fs::read_to_string(&abs_path).unwrap();
    assert!(file_content.contains("derived_from"));
    assert!(file_content.contains("a1b2c3d4-e5f6-4789-abcd-000000000061"));
}

#[test]
fn promotion_dry_run_makes_no_changes() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000061",
                "observation",
                "Obs 1",
                "c",
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
                    target_id: "a1b2c3d4-e5f6-4789-abcd-000000000061".to_string(),
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
        let mut e1 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000061",
            "observation",
            "A",
            "content",
        );
        e1.created_at = "2026-01-01T00:00:00Z".to_string();
        // Relative path as stored by sync.rs (relative to .cogz).
        e1.file_path =
            Some("observations/2026-01/a1b2c3d4-e5f6-4789-abcd-000000000061.md".to_string());
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000061",
            "observation",
            &embedding,
        )
        .unwrap();

        let mut e2 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000062",
            "observation",
            "B",
            "content",
        );
        e2.created_at = "2026-02-01T00:00:00Z".to_string();
        e2.file_path =
            Some("observations/2026-01/a1b2c3d4-e5f6-4789-abcd-000000000062.md".to_string());
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000062",
            "observation",
            &embedding,
        )
        .unwrap();

        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000063",
                "knowledge",
                "K",
                "c",
            ),
        )
        .unwrap();
        insert_edge(
            &conn,
            &Edge {
                source_id: "a1b2c3d4-e5f6-4789-abcd-000000000063".to_string(),
                target_id: "a1b2c3d4-e5f6-4789-abcd-000000000062".to_string(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    write_observation_file(
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000061",
        "A",
        "2026-01-01T00:00:00Z",
    );
    write_observation_file(
        &cogz_dir,
        "a1b2c3d4-e5f6-4789-abcd-000000000062",
        "B",
        "2026-02-01T00:00:00Z",
    );

    let results = run_merge(&storage, &cogz_dir, &config(), None, false).unwrap();
    assert_eq!(results.len(), 1);

    let conn = storage.conn();
    let e2 =
        cogz::storage::crud::get_entity(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000062").unwrap();
    assert_eq!(e2.status, "superseded");

    // The superseded_by edge connects obs-2 to obs-1 (canonical).
    let edges_from = get_edges_from(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000062").unwrap();
    assert!(
        edges_from.iter().any(|e| e.edge_type == "superseded_by"
            && e.target_id == "a1b2c3d4-e5f6-4789-abcd-000000000061")
    );

    // Original edges to obs-2 are preserved (not redirected).
    // Graph expansion follows superseded_by to reach the survivor.
    let old_edges = get_edges_to(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000062").unwrap();
    assert!(
        old_edges
            .iter()
            .any(|e| e.source_id == "a1b2c3d4-e5f6-4789-abcd-000000000063"
                && e.edge_type == "references")
    );
}

#[test]
fn merge_dry_run_makes_no_changes() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");

    let embedding = vec![0.1_f32; 768];
    {
        let conn = storage.conn();
        let mut e1 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000061",
            "observation",
            "A",
            "content",
        );
        e1.created_at = "2026-01-01T00:00:00Z".to_string();
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000061",
            "observation",
            &embedding,
        )
        .unwrap();

        let mut e2 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000062",
            "observation",
            "B",
            "content",
        );
        e2.created_at = "2026-02-01T00:00:00Z".to_string();
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000062",
            "observation",
            &embedding,
        )
        .unwrap();
    }

    let results = run_merge(&storage, &cogz_dir, &config(), None, true).unwrap();
    assert_eq!(results.len(), 1);

    let conn = storage.conn();
    let e2 =
        cogz::storage::crud::get_entity(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000062").unwrap();
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
    let merged = run_merge(&storage, &cogz_dir, &config(), None, false).unwrap();
    assert!(promoted.is_empty());
    assert!(merged.is_empty());
}

#[test]
fn dedup_works_without_embeddings() {
    let storage = setup();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000051",
            "observation",
            "Same title",
            "c",
        ),
    )
    .unwrap();

    let result = check_duplicate(
        &conn,
        "a1b2c3d4-e5f6-4789-abcd-000000000052",
        "Same title",
        "observation",
        None,
        &config(),
    );
    assert!(result.duplicate_warning.is_some());
    assert!(!result.dedup_flagged);
}
