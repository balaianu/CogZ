use super::*;
use crate::files::frontmatter::Frontmatter;
use crate::storage::crud::{Entity, insert_entity};
use crate::storage::edges::{Edge, insert_edge};
use crate::storage::embeddings::insert_embedding;
use crate::storage::ensure_vec_extension;

fn setup() -> crate::storage::Storage {
    ensure_vec_extension();
    crate::storage::Storage::open_memory().unwrap()
}

fn config() -> ConsolidationConfig {
    ConsolidationConfig {
        dedup_threshold: 0.92,
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
fn dry_run_reports_candidates_without_changes() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(cogz_dir.join("observations").join("2026-01")).unwrap();

    let embedding = vec![0.1_f32; 768];
    {
        let conn = storage.conn();
        let mut e1 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000041",
            "observation",
            "A",
            "content",
        );
        e1.created_at = "2026-01-01T00:00:00Z".to_string();
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000041",
            "observation",
            &embedding,
        )
        .unwrap();

        let mut e2 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000042",
            "observation",
            "B",
            "content",
        );
        e2.created_at = "2026-02-01T00:00:00Z".to_string();
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000042",
            "observation",
            &embedding,
        )
        .unwrap();

        // Write observation files so merge_one could read them.
        for (id, title) in [
            ("a1b2c3d4-e5f6-4789-abcd-000000000041", "A"),
            ("a1b2c3d4-e5f6-4789-abcd-000000000042", "B"),
        ] {
            let path = cogz_dir
                .join("observations")
                .join("2026-01")
                .join(format!("{id}.md"));
            let mut fm = Frontmatter::new();
            fm.insert("id", FmValue::String(id.to_string()));
            fm.insert("title", FmValue::String(title.to_string()));
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
            std::fs::write(
                &path,
                format!(
                    "---\n{}---\n\ncontent",
                    crate::files::frontmatter::serialize(&fm)
                ),
            )
            .unwrap();
        }
    }

    let results = run_merge(&storage, &cogz_dir, &config(), true).unwrap();
    assert_eq!(results.len(), 1);
    // obs-1 created first → survivor.
    assert_eq!(
        results[0].survivor_id,
        "a1b2c3d4-e5f6-4789-abcd-000000000041"
    );
    assert_eq!(
        results[0].superseded_id,
        "a1b2c3d4-e5f6-4789-abcd-000000000042"
    );

    // No status change.
    let conn = storage.conn();
    let e2 =
        crate::storage::crud::get_entity(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000042").unwrap();
    assert_eq!(e2.status, "active");
}

#[test]
fn merge_redirects_edges_and_marks_superseded() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    let obs_dir = cogz_dir.join("observations").join("2026-01");
    std::fs::create_dir_all(&obs_dir).unwrap();

    let embedding = vec![0.1_f32; 768];
    {
        let conn = storage.conn();
        let mut e1 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000041",
            "observation",
            "A",
            "content",
        );
        e1.created_at = "2026-01-01T00:00:00Z".to_string();
        // Relative path as stored by sync.rs (relative to .cogz).
        e1.file_path =
            Some("observations/2026-01/a1b2c3d4-e5f6-4789-abcd-000000000041.md".to_string());
        insert_entity(&conn, &e1).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000041",
            "observation",
            &embedding,
        )
        .unwrap();

        let mut e2 = Entity::new(
            "a1b2c3d4-e5f6-4789-abcd-000000000042",
            "observation",
            "B",
            "content",
        );
        e2.created_at = "2026-02-01T00:00:00Z".to_string();
        e2.file_path =
            Some("observations/2026-01/a1b2c3d4-e5f6-4789-abcd-000000000042.md".to_string());
        insert_entity(&conn, &e2).unwrap();
        insert_embedding(
            &conn,
            "a1b2c3d4-e5f6-4789-abcd-000000000042",
            "observation",
            &embedding,
        )
        .unwrap();

        // A third entity references obs-2.
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000043",
                "knowledge",
                "K",
                "c",
            ),
        )
        .unwrap();
        insert_edge(
            &conn,
            &Edge {
                source_id: "a1b2c3d4-e5f6-4789-abcd-000000000043".to_string(),
                target_id: "a1b2c3d4-e5f6-4789-abcd-000000000042".to_string(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();

        // Write observation files.
        for (id, title, created) in [
            (
                "a1b2c3d4-e5f6-4789-abcd-000000000041",
                "A",
                "2026-01-01T00:00:00Z",
            ),
            (
                "a1b2c3d4-e5f6-4789-abcd-000000000042",
                "B",
                "2026-02-01T00:00:00Z",
            ),
        ] {
            let path = obs_dir.join(format!("{id}.md"));
            let mut fm = Frontmatter::new();
            fm.insert("id", FmValue::String(id.to_string()));
            fm.insert("title", FmValue::String(title.to_string()));
            fm.insert("type", FmValue::String("observation".to_string()));
            fm.insert("status", FmValue::String("active".to_string()));
            fm.insert("created_at", FmValue::String(created.to_string()));
            fm.insert("updated_at", FmValue::String(created.to_string()));
            fm.insert("references", FmValue::Array(vec![]));
            std::fs::write(
                &path,
                format!(
                    "---\n{}---\n\ncontent",
                    crate::files::frontmatter::serialize(&fm)
                ),
            )
            .unwrap();
        }
    }

    let results = run_merge(&storage, &cogz_dir, &config(), false).unwrap();
    assert_eq!(results.len(), 1);

    let conn = storage.conn();
    // obs-2 should be superseded.
    let e2 =
        crate::storage::crud::get_entity(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000042").unwrap();
    assert_eq!(e2.status, "superseded");
    assert_eq!(
        e2.properties["superseded_by"],
        serde_json::Value::String("a1b2c3d4-e5f6-4789-abcd-000000000041".to_string())
    );

    // The superseded_by edge should connect obs-2 to obs-1.
    let edges_from_superseded =
        crate::storage::edges::get_edges_from(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000042")
            .unwrap();
    assert!(
        edges_from_superseded
            .iter()
            .any(|e| e.edge_type == "superseded_by"
                && e.target_id == "a1b2c3d4-e5f6-4789-abcd-000000000041")
    );

    // The original references edge from kn-1 to obs-2 is preserved
    // (not redirected). Graph expansion follows superseded_by to
    // reach the survivor.
    let edges_to_superseded =
        crate::storage::edges::get_edges_to(&conn, "a1b2c3d4-e5f6-4789-abcd-000000000042").unwrap();
    assert!(
        edges_to_superseded
            .iter()
            .any(|e| e.source_id == "a1b2c3d4-e5f6-4789-abcd-000000000043"
                && e.edge_type == "references")
    );
}

#[test]
fn no_merge_without_embeddings() {
    let storage = setup();
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");

    {
        let conn = storage.conn();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000041",
                "observation",
                "A",
                "c",
            ),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new(
                "a1b2c3d4-e5f6-4789-abcd-000000000042",
                "observation",
                "B",
                "c",
            ),
        )
        .unwrap();
    }

    let results = run_merge(&storage, &cogz_dir, &config(), false).unwrap();
    assert!(results.is_empty());
}
