//! Integration tests for Phase 12 doctor and pruning.
//!
//! Tests cover:
//! - Doctor reports healthy on a freshly indexed repo
//! - Doctor detects missing files (DB entity exists, file deleted)
//! - Doctor detects observation content edits (append-only violation)
//! - Doctor detects orphaned supersedes
//! - Prune dry-run reports candidates correctly
//! - Prune confirm creates tombstones, deletes files
//! - Active observations are never pruned
//! - Tombstone limit is enforced

use std::sync::Arc;

use chrono::{Duration, Utc};
use cogz::config::Config;
use cogz::doctor::checks::{IssueKind, run_doctor};
use cogz::doctor::prune::{find_prune_candidates, run_prune};
use cogz::files::{EntityFile, FileEntityType, FmValue, sync_all, write_entity_file};
use cogz::storage::Storage;
use cogz::storage::crud::update_status;
use cogz::storage::edges::{Edge, insert_edge};

fn setup() -> (Arc<Storage>, Config, std::path::PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(&cogz_dir).unwrap();
    std::fs::create_dir_all(cogz_dir.join("observations/2026-08")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("knowledge")).unwrap();

    let storage = Arc::new(Storage::open_memory().unwrap());
    let config = Config::default_for("test-doctor");
    (storage, config, cogz_dir, dir)
}

fn make_observation(title: &str, content: &str) -> EntityFile {
    let mut entity = EntityFile::new(title, FileEntityType::Observation, content);
    entity
        .frontmatter
        .insert("source", FmValue::String("test".to_string()));
    entity
}

fn make_knowledge(title: &str, content: &str) -> EntityFile {
    EntityFile::new(title, FileEntityType::Knowledge, content)
}

fn write_and_sync(
    storage: &Storage,
    cogz_dir: &std::path::Path,
    entity: &EntityFile,
    subdir: &str,
) {
    let path = cogz_dir.join(subdir).join(format!("{}.md", entity.id));
    write_entity_file(&path, entity).unwrap();
    let _ = sync_all(storage, cogz_dir);
}

fn make_old_and_rejected(storage: &Storage, id: &str, days: i64) {
    let conn = storage.conn();
    update_status(&conn, id, "rejected").unwrap();
    let old_date = (Utc::now() - Duration::days(days)).to_rfc3339();
    conn.execute(
        "UPDATE entities SET updated_at = ?1 WHERE id = ?2",
        [old_date, id.to_string()],
    )
    .unwrap();
}

#[test]
fn doctor_healthy_on_fresh_index() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Test obs", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");

    let report = run_doctor(&storage, &config, &cogz_dir, &cogz_dir);

    assert!(report.db_healthy);
    let health_issues: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind != IssueKind::MissingEntity)
        .collect();
    assert!(
        health_issues.is_empty(),
        "expected no health issues, got: {:?}",
        health_issues
    );
}

#[test]
fn doctor_detects_missing_file() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Test obs", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");

    let file_path = cogz_dir
        .join("observations/2026-08")
        .join(format!("{}.md", obs.id));
    std::fs::remove_file(&file_path).unwrap();

    let report = run_doctor(&storage, &config, &cogz_dir, &cogz_dir);

    let missing: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::MissingFile)
        .collect();
    assert!(!missing.is_empty(), "expected missing_file issue");
}

#[test]
fn doctor_detects_observation_edit() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Test obs", "original content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");

    let file_path = cogz_dir
        .join("observations/2026-08")
        .join(format!("{}.md", obs.id));
    let edited = make_observation("Test obs", "EDITED content");
    // Keep the same ID so the file path matches.
    let mut edited = edited;
    edited.id = obs.id.clone();
    write_entity_file(&file_path, &edited).unwrap();

    let report = run_doctor(&storage, &config, &cogz_dir, &cogz_dir);

    let edited_issues: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::ObservationEdited)
        .collect();
    assert!(
        !edited_issues.is_empty(),
        "expected observation_edited issue"
    );
}

#[test]
fn doctor_detects_orphaned_supersede() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Test obs", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");

    {
        let conn = storage.conn();
        update_status(&conn, &obs.id, "superseded").unwrap();
    }

    let report = run_doctor(&storage, &config, &cogz_dir, &cogz_dir);

    let orphaned: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::OrphanedSupersede)
        .collect();
    assert!(!orphaned.is_empty(), "expected orphaned_supersede issue");
}

#[test]
fn prune_dry_run_reports_candidates() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Old obs", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");
    make_old_and_rejected(&storage, &obs.id, 100);

    let candidates = find_prune_candidates(&storage, &config);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].entity_id, obs.id);
    assert_eq!(candidates[0].status, "rejected");
}

#[test]
fn prune_confirm_deletes_file_and_creates_tombstone() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Old obs", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");
    make_old_and_rejected(&storage, &obs.id, 100);

    let file_path = cogz_dir
        .join("observations/2026-08")
        .join(format!("{}.md", obs.id));

    let report = run_prune(&storage, &config, &cogz_dir, true);

    assert_eq!(report.pruned, 1);
    assert!(!file_path.exists(), "observation file should be deleted");

    let conn = storage.conn();
    let status: String = conn
        .query_row(
            "SELECT status FROM entities WHERE id = ?1",
            [&obs.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "pruned");
}

#[test]
fn prune_never_prunes_active_observations() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Active obs", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");

    {
        let conn = storage.conn();
        let old_date = (Utc::now() - Duration::days(100)).to_rfc3339();
        conn.execute(
            "UPDATE entities SET updated_at = ?1 WHERE id = ?2",
            [old_date, obs.id.clone()],
        )
        .unwrap();
    }

    let candidates = find_prune_candidates(&storage, &config);
    assert!(
        candidates.is_empty(),
        "active observations should not be prunable"
    );
}

#[test]
fn prune_never_prunes_knowledge_or_rules() {
    let (storage, config, cogz_dir, _dir) = setup();

    let knowledge = make_knowledge("Test knowledge", "content");
    write_and_sync(&storage, &cogz_dir, &knowledge, "knowledge");

    {
        let conn = storage.conn();
        let old_date = (Utc::now() - Duration::days(100)).to_rfc3339();
        conn.execute(
            "UPDATE entities SET updated_at = ?1, status = 'rejected' WHERE id = ?2",
            [old_date, knowledge.id.clone()],
        )
        .unwrap();
    }

    let candidates = find_prune_candidates(&storage, &config);
    assert!(candidates.is_empty(), "knowledge should not be prunable");
}

#[test]
fn prune_preserves_graph_edges_to_tombstone() {
    let (storage, config, cogz_dir, _dir) = setup();

    let obs = make_observation("Old obs", "content");
    let knowledge = make_knowledge("Linked knowledge", "content");
    write_and_sync(&storage, &cogz_dir, &obs, "observations/2026-08");
    write_and_sync(&storage, &cogz_dir, &knowledge, "knowledge");

    {
        let conn = storage.conn();
        let edge = Edge {
            source_id: knowledge.id.clone(),
            target_id: obs.id.clone(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: Utc::now().to_rfc3339(),
        };
        insert_edge(&conn, &edge).unwrap();
    }

    make_old_and_rejected(&storage, &obs.id, 100);

    let _report = run_prune(&storage, &config, &cogz_dir, true);

    let conn = storage.conn();
    let edge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE target_id = ?1",
            [&obs.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edge_count, 1, "edge to tombstone should be preserved");
}

#[test]
fn tombstone_limit_enforced() {
    let (storage, config, cogz_dir, _dir) = setup();

    // Write all observation files first, then sync once.
    let mut ids: Vec<String> = Vec::new();
    for i in 0..5 {
        let obs = make_observation(&format!("Old obs {}", i), "content");
        let path = cogz_dir
            .join("observations/2026-08")
            .join(format!("{}.md", obs.id));
        write_entity_file(&path, &obs).unwrap();
        ids.push(obs.id);
    }
    let _ = sync_all(&storage, &cogz_dir);

    // Now update all statuses to rejected and make them old.
    for (i, id) in ids.iter().enumerate() {
        make_old_and_rejected(&storage, id, 100 + i as i64);
    }

    let mut config = config;
    config.retention.tombstone_max_count = 2;

    let report = run_prune(&storage, &config, &cogz_dir, true);

    assert_eq!(report.pruned, 5);
    assert_eq!(
        report.tombstones_removed, 3,
        "should remove 3 oldest tombstones"
    );

    let conn = storage.conn();
    let tombstone_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE status = 'pruned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tombstone_count, 2);
}
