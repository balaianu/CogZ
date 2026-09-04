//! Integration tests for file → DB synchronization.
//!
//! Tests the full file sync pipeline: create files, index, verify DB
//! entities; edit files, reindex, verify updates; delete files,
//! reindex, verify stale marking; verify rebuildability.

use std::path::{Path, PathBuf};

use cogz::files::{content_hash, scan_entity_files, sync_all, sync_incremental, sync_single_file};
use cogz::storage::{self, Storage};

fn setup_storage() -> Storage {
    Storage::open_memory().unwrap()
}

fn write_file(dir: &Path, rel_path: &str, content: &str) -> PathBuf {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

fn knowledge_file(id: &str, title: &str, category: &str, body: &str) -> String {
    format!(
        "---\nid: {}\ntitle: \"{}\"\ntype: knowledge\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: []\ncategory: {}\n---\n\n{}",
        id, title, category, body
    )
}

fn rule_file(id: &str, title: &str, body: &str) -> String {
    format!(
        "---\nid: {}\ntitle: \"{}\"\ntype: rule\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: []\n---\n\n{}",
        id, title, body
    )
}

fn observation_file(id: &str, title: &str, body: &str) -> String {
    format!(
        "---\nid: {}\ntitle: \"{}\"\ntype: observation\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: []\nsource: agent\nconfidence: 0.5\n---\n\n{}",
        id, title, body
    )
}

#[test]
fn content_hash_is_deterministic() {
    let h1 = content_hash("hello world");
    let h2 = content_hash("hello world");
    assert_eq!(h1, h2);
}

#[test]
fn content_hash_differs_for_different_content() {
    let h1 = content_hash("hello");
    let h2 = content_hash("world");
    assert_ne!(h1, h2);
}

#[test]
fn scan_finds_entity_files() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "knowledge/arch/test.md", "content");
    write_file(dir.path(), "rules/test-rule.md", "content");
    write_file(dir.path(), "observations/2026-08/abc.md", "content");
    write_file(dir.path(), "knowledge/arch/notmd.txt", "content");

    let files = scan_entity_files(dir.path());
    assert_eq!(files.len(), 3); // notmd.txt excluded
}

#[test]
fn sync_creates_new_entities() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "System overview"),
    );
    write_file(
        dir.path(),
        "rules/use-sqlite.md",
        &rule_file("r1-uuid", "Use SQLite", "Always use SQLite"),
    );

    let result = sync_all(&storage, dir.path());

    assert_eq!(result.created, 2);
    assert_eq!(result.errors.len(), 0);

    let conn = storage.conn();
    assert_eq!(storage::crud::count_all(&conn).unwrap(), 2);
}

#[test]
fn sync_updates_changed_files() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    let path = write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "Original content"),
    );

    // First sync creates
    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 1);

    // Modify the file
    std::fs::write(
        &path,
        knowledge_file("k1-uuid", "Overview", "architecture", "Updated content"),
    )
    .unwrap();

    // Second sync updates
    let result = sync_all(&storage, dir.path());
    assert_eq!(result.updated, 1);
    assert_eq!(result.created, 0);

    let conn = storage.conn();
    let entity = storage::crud::get_entity(&conn, "k1-uuid").unwrap();
    assert_eq!(entity.content, "Updated content");
}

#[test]
fn sync_incremental_skips_unchanged() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "Content"),
    );

    // First sync creates
    let result = sync_incremental(&storage, dir.path());
    assert_eq!(result.created, 1);

    // Second sync skips (hash unchanged)
    let result = sync_incremental(&storage, dir.path());
    assert_eq!(result.skipped, 1);
    assert_eq!(result.created, 0);
    assert_eq!(result.updated, 0);
}

#[test]
fn sync_marks_deleted_files_stale() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    let path = write_file(
        dir.path(),
        "rules/test-rule.md",
        &rule_file("r1-uuid", "Test Rule", "Rule body"),
    );

    // First sync creates
    sync_all(&storage, dir.path());

    // Delete the file
    std::fs::remove_file(&path).unwrap();

    // Second sync marks stale
    let result = sync_all(&storage, dir.path());
    assert_eq!(result.marked_stale, 1);

    let conn = storage.conn();
    let entity = storage::crud::get_entity(&conn, "r1-uuid").unwrap();
    assert_eq!(entity.status, "stale");
}

#[test]
fn sync_preserves_edges_on_stale() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    // Create two files, one referencing the other
    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "Content"),
    );
    write_file(
        dir.path(),
        "rules/ref-rule.md",
        "---\nid: r1-uuid\ntitle: \"Ref Rule\"\ntype: rule\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: [\"k1-uuid\"]\n---\n\nRule body",
    );

    sync_all(&storage, dir.path());

    {
        let conn = storage.conn();
        assert_eq!(storage::edges::count_edges(&conn).unwrap(), 1);
    }

    // Delete the knowledge file
    std::fs::remove_file(dir.path().join("knowledge/architecture/overview.md")).unwrap();
    sync_all(&storage, dir.path());

    // Edge should still exist (stale entity preserved)
    {
        let conn = storage.conn();
        assert_eq!(storage::edges::count_edges(&conn).unwrap(), 1);
    }
}

#[test]
fn sync_references_edges() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "Content"),
    );
    write_file(
        dir.path(),
        "rules/ref-rule.md",
        "---\nid: r1-uuid\ntitle: \"Ref Rule\"\ntype: rule\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: [\"k1-uuid\"]\n---\n\nRule body",
    );

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 2);
    assert_eq!(result.errors.len(), 0);

    let conn = storage.conn();
    let edges = storage::edges::get_edges_from(&conn, "r1-uuid").unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].target_id, "k1-uuid");
    assert_eq!(edges[0].edge_type, "references");
}

#[test]
fn sync_forward_reference_edge_created() {
    // Knowledge is processed before rules (ENTITY_DIRS order).
    // A knowledge file referencing a rule is a forward reference —
    // the rule doesn't exist yet when the knowledge is synced.
    // The second pass must create the edge.
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        "---\nid: k1-uuid\ntitle: \"Overview\"\ntype: knowledge\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: [\"r1-uuid\"]\ncategory: architecture\n---\n\nContent",
    );
    write_file(
        dir.path(),
        "rules/ref-rule.md",
        "---\nid: r1-uuid\ntitle: \"Ref Rule\"\ntype: rule\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: []\n---\n\nRule body",
    );

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 2);
    assert_eq!(result.errors.len(), 0);

    let conn = storage.conn();
    let edges = storage::edges::get_edges_from(&conn, "k1-uuid").unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].target_id, "r1-uuid");
    assert_eq!(edges[0].edge_type, "references");
}

#[test]
fn sync_incremental_restores_forward_ref_to_new_entity() {
    // Entity A exists and references entity B (which doesn't exist yet).
    // First sync: A is created, B is missing — A's reference edge is
    // skipped (FK constraint). Second sync (incremental): B is added.
    // A is unchanged (hash matches, skipped). The reference edge from
    // A to B must be created in the second sync's reference pass.
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    // Create A with a reference to B (B doesn't exist yet).
    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        "---\nid: k1-uuid\ntitle: \"Overview\"\ntype: knowledge\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: [\"r1-uuid\"]\ncategory: architecture\n---\n\nContent",
    );

    // First sync — A is created, reference to B is skipped (FK).
    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 1);
    assert_eq!(result.errors.len(), 0);

    // No edge yet — B doesn't exist.
    {
        let conn = storage.conn();
        let edges = storage::edges::get_edges_from(&conn, "k1-uuid").unwrap();
        assert!(edges.is_empty(), "no edges expected — target doesn't exist");
    }

    // Now create B.
    write_file(
        dir.path(),
        "rules/ref-rule.md",
        "---\nid: r1-uuid\ntitle: \"Ref Rule\"\ntype: rule\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: []\n---\n\nRule body",
    );

    // Second sync — incremental. A is unchanged (skipped), B is new.
    // The reference pass must re-sync A's references and create the edge.
    let result = sync_incremental(&storage, dir.path());
    assert_eq!(result.created, 1);
    assert_eq!(result.errors.len(), 0);

    // Edge from A to B should now exist.
    {
        let conn = storage.conn();
        let edges = storage::edges::get_edges_from(&conn, "k1-uuid").unwrap();
        assert_eq!(edges.len(), 1, "forward reference edge must be created");
        assert_eq!(edges[0].target_id, "r1-uuid");
        assert_eq!(edges[0].edge_type, "references");
    }
}

#[test]
fn sync_supports_edges_from_supporting_ids() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    // Target observation that will be supported.
    write_file(
        dir.path(),
        "observations/2026-01/target.md",
        "---\nid: target-uuid\ntitle: \"Target Obs\"\ntype: observation\nstatus: active\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\nreferences: []\nsource: agent\nconfidence: 0.5\n---\n\nTarget content",
    );
    // Supporting observation with supporting_ids pointing to target.
    write_file(
        dir.path(),
        "observations/2026-01/supporter.md",
        "---\nid: supporter-uuid\ntitle: \"Supporter\"\ntype: observation\nstatus: active\ncreated_at: 2026-01-02T00:00:00Z\nupdated_at: 2026-01-02T00:00:00Z\nreferences: []\nsupporting_ids: [\"target-uuid\"]\nsource: agent\nconfidence: 0.5\n---\n\nSupporting content",
    );

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 2);
    assert_eq!(result.errors.len(), 0);

    let conn = storage.conn();
    let edges = storage::edges::get_edges_from(&conn, "supporter-uuid").unwrap();
    let supports_edges: Vec<_> = edges.iter().filter(|e| e.edge_type == "supports").collect();
    assert_eq!(supports_edges.len(), 1);
    assert_eq!(supports_edges[0].target_id, "target-uuid");
}

#[test]
fn sync_observations_with_properties() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "observations/2026-08/obs-uuid.md",
        &observation_file("obs-uuid", "Test Bug", "Found a bug"),
    );

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 1);

    let conn = storage.conn();
    let entity = storage::crud::get_entity(&conn, "obs-uuid").unwrap();
    assert_eq!(entity.r#type, "observation");
    assert_eq!(entity.properties["source"], "agent");
    assert_eq!(entity.properties["confidence"], 0.5);
}

#[test]
fn sync_finds_files_in_subdirectories() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "knowledge/decisions/use-sqlite.md",
        &knowledge_file("k1", "Use SQLite", "decisions", "Content"),
    );
    write_file(
        dir.path(),
        "knowledge/gotchas/fts5-bug.md",
        &knowledge_file("k2", "FTS5 Bug", "gotchas", "Content"),
    );

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 2);
}

#[test]
fn sync_resets_db_rebuilds_from_files() {
    // The core invariant: reset + index rebuilds everything
    let dir = tempfile::tempdir().unwrap();

    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "Content"),
    );
    write_file(
        dir.path(),
        "rules/test-rule.md",
        &rule_file("r1-uuid", "Test Rule", "Rule body"),
    );

    // First sync
    {
        let storage = Storage::open(&dir.path().join("test.db"), 768).unwrap();
        sync_all(&storage, dir.path());
        let conn = storage.conn();
        assert_eq!(storage::crud::count_all(&conn).unwrap(), 2);
    }

    // Drop the DB (simulating cogz reset)
    std::fs::remove_file(dir.path().join("test.db")).unwrap();

    // Rebuild from files
    {
        let storage = Storage::open(&dir.path().join("test.db"), 768).unwrap();
        let result = sync_all(&storage, dir.path());
        assert_eq!(result.created, 2);
        let conn = storage.conn();
        assert_eq!(storage::crud::count_all(&conn).unwrap(), 2);

        // Verify content is correct
        let k = storage::crud::get_entity(&conn, "k1-uuid").unwrap();
        assert_eq!(k.content, "Content");
    }
}

#[test]
fn sync_handles_empty_cogz_dir() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.created, 0);
    assert_eq!(result.errors.len(), 0);
}

#[test]
fn sync_reports_errors_for_bad_files() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    write_file(dir.path(), "rules/bad.md", "not valid frontmatter");

    let result = sync_all(&storage, dir.path());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.created, 0);
}

#[test]
fn sync_records_create_and_edit_events() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    let path = write_file(
        dir.path(),
        "rules/test-rule.md",
        &rule_file("r1-uuid", "Test Rule", "Original body"),
    );

    // First sync — should record a rule_created event
    sync_all(&storage, dir.path());
    {
        let conn = storage.conn();
        let events = storage::events::get_events_for_entity(&conn, "r1-uuid").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "rule_created");
    }

    // Edit the body — should record a rule_edited event
    std::fs::write(&path, rule_file("r1-uuid", "Test Rule", "Updated body")).unwrap();
    sync_all(&storage, dir.path());
    {
        let conn = storage.conn();
        let events = storage::events::get_events_for_entity(&conn, "r1-uuid").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "rule_edited");
        assert_eq!(events[1].event_type, "rule_created");
    }
}

#[test]
fn sync_single_file_syncs_reference_edges() {
    let storage = setup_storage();
    let dir = tempfile::tempdir().unwrap();

    // Create a knowledge file with no references.
    write_file(
        dir.path(),
        "knowledge/architecture/overview.md",
        &knowledge_file("k1-uuid", "Overview", "architecture", "Content"),
    );

    // Initial full sync.
    sync_all(&storage, dir.path());

    // Verify no references edge.
    {
        let conn = storage.conn();
        let edges = storage::edges::get_edges_from(&conn, "k1-uuid").unwrap();
        assert!(edges.is_empty(), "no references expected initially");
    }

    // Edit the file to add a reference to a new entity.
    let target_content = knowledge_file("k2-uuid", "Target", "architecture", "Target content");
    write_file(
        dir.path(),
        "knowledge/architecture/target.md",
        &target_content,
    );

    // Sync the target first so the FK constraint is satisfied.
    sync_all(&storage, dir.path());

    // Now edit the source file to add a reference to k2-uuid.
    let updated_source = "---\nid: k1-uuid\ntitle: \"Overview\"\ntype: knowledge\nstatus: active\ncreated_at: 2026-08-27T14:30:00Z\nupdated_at: 2026-08-27T14:30:00Z\nreferences: [\"k2-uuid\"]\ncategory: architecture\n---\n\nContent";
    let source_path = dir.path().join("knowledge/architecture/overview.md");
    std::fs::write(&source_path, updated_source).unwrap();

    // Sync only the single edited file (simulating the file_save hook).
    let result = sync_single_file(&storage, dir.path(), "knowledge/architecture/overview.md");
    assert_eq!(result.errors.len(), 0, "no errors expected");
    assert_eq!(result.updated, 1, "file should be updated");

    // Verify the references edge was created.
    {
        let conn = storage.conn();
        let edges = storage::edges::get_edges_from(&conn, "k1-uuid").unwrap();
        assert_eq!(edges.len(), 1, "one references edge expected");
        assert_eq!(edges[0].edge_type, "references");
        assert_eq!(edges[0].target_id, "k2-uuid");
    }
}
