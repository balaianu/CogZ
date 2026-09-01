//! Stale knowledge flagging — when code entities change, observations
//! and rules referencing them are marked `status = 'stale'`.
//!
//! This implements the file-first invariant: the entity's frontmatter
//! `status` field is updated on disk before the DB is touched, matching
//! the pattern used by consolidation (`merge_one`, `record_contradictions`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::files::{read_entity_file, write_entity_file};
use crate::storage::events::{EventType, record_event};
use crate::storage::graph::get_edges_involving_batch;
use crate::storage::status::transition_status;
use crate::storage::{self, Storage};

/// Find entities with `references` edges to any of the changed code
/// entity IDs. Returns the referencing entity IDs (deduplicated).
///
/// Only `active` entities are considered — already-stale, rejected, or
/// superseded entities are not touched.
fn find_referencing_entities(
    conn: &rusqlite::Connection,
    changed_code_ids: &[String],
) -> Vec<String> {
    if changed_code_ids.is_empty() {
        return Vec::new();
    }

    let edges = match get_edges_involving_batch(conn, changed_code_ids) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("failed to query edges for stale flagging: {}", e);
            return Vec::new();
        }
    };

    let changed_set: HashSet<&str> = changed_code_ids.iter().map(|s| s.as_str()).collect();

    let mut referencing: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (source_id, target_id, edge_type) in &edges {
        if edge_type == "references"
            && changed_set.contains(target_id.as_str())
            && seen.insert(source_id.clone())
        {
            referencing.push(source_id.clone());
        }
    }

    referencing
}

/// Resolve a DB-stored file path to an absolute path.
///
/// DB paths for file-backed entities are stored relative to `.cogz`
/// (e.g. `observations/2026-08/uuid.md`).
fn resolve_file_path(file_path: &str, cogz_dir: &Path) -> PathBuf {
    if file_path.starts_with(".cogz") {
        cogz_dir.parent().unwrap_or(cogz_dir).join(file_path)
    } else {
        cogz_dir.join(file_path)
    }
}

/// Mark observations/rules/knowledge that reference changed code
/// entities as stale. Follows the file-first invariant: updates the
/// frontmatter `status` field on disk before updating the DB.
///
/// Only `active` entities are marked stale. Already-stale or terminal
/// entities are skipped.
///
/// Records a single `code_changed` event with a payload listing the
/// affected entity IDs.
///
/// Returns the number of entities marked stale.
pub fn flag_stale_knowledge(
    storage: &Storage,
    cogz_dir: &Path,
    changed_code_ids: &[String],
) -> usize {
    if changed_code_ids.is_empty() {
        return 0;
    }

    let conn = storage.conn();

    let referencing_ids = find_referencing_entities(&conn, changed_code_ids);
    if referencing_ids.is_empty() {
        return 0;
    }

    // Batch-fetch only the referencing entities, then filter by type
    // and status in Rust. This avoids 3 queries scanning up to 100k
    // entities each when the referencing set is typically 1-10 IDs.
    let knowledge_types = ["observation", "rule", "knowledge"];
    let entities = match storage::crud::get_entities_batch(&conn, &referencing_ids) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("failed to batch-fetch entities for stale flagging: {}", e);
            return 0;
        }
    };

    let mut to_flag: Vec<(storage::crud::Entity, PathBuf)> = Vec::new();

    for entity in entities {
        if !knowledge_types.contains(&entity.r#type.as_str()) {
            continue;
        }
        if entity.status != "active" {
            continue;
        }
        let Some(file_path) = entity.file_path.clone() else {
            tracing::warn!("entity {} has no file_path, cannot flag stale", entity.id);
            continue;
        };
        to_flag.push((entity, resolve_file_path(&file_path, cogz_dir)));
    }

    drop(conn);

    let mut flagged_ids: Vec<String> = Vec::new();

    for (entity, path) in &to_flag {
        // 1. Read the file, update frontmatter status, write it back.
        let mut entity_file = match read_entity_file(path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("failed to read {}: {}", path.display(), e);
                continue;
            }
        };

        entity_file.status = "stale".to_string();
        entity_file.updated_at = chrono::Utc::now().to_rfc3339();

        if let Err(e) = write_entity_file(path, &entity_file) {
            tracing::warn!("failed to write {}: {}", path.display(), e);
            continue;
        }

        // 2. Validate the transition and update the DB.
        if let Err(e) = transition_status(&entity.status, "stale") {
            tracing::warn!("illegal status transition for {}: {}", entity.id, e);
            // The file was already written — revert it.
            entity_file.status = entity.status.clone();
            let _ = write_entity_file(path, &entity_file);
            continue;
        }

        let conn = storage.conn();
        let mut updated = entity.clone();
        updated.status = "stale".to_string();
        updated.updated_at = entity_file.updated_at.clone();

        if let Err(e) = storage::crud::update_entity(&conn, &updated) {
            tracing::warn!("failed to update DB status for {}: {}", entity.id, e);
        } else {
            flagged_ids.push(entity.id.clone());
        }
        drop(conn);
    }

    // 3. Record a single code_changed event with affected entity IDs.
    if !flagged_ids.is_empty() {
        let conn = storage.conn();
        let payload = serde_json::json!({
            "affected_entities": flagged_ids,
            "changed_code_ids": changed_code_ids,
            "count": flagged_ids.len(),
        });
        if let Err(e) = record_event(&conn, EventType::CodeChanged, None, &payload) {
            tracing::warn!("failed to record code_changed event: {}", e);
        }
    }

    flagged_ids.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::{EntityFile, FileEntityType, write_entity_file};
    use crate::storage::crud::{Entity, insert_entity};
    use crate::storage::edges::{Edge, insert_edge};
    use tempfile::TempDir;

    fn setup() -> (TempDir, Storage) {
        let dir = tempfile::tempdir().unwrap();
        let cogz_dir = dir.path().join(".cogz");
        std::fs::create_dir_all(cogz_dir.join("observations/2026-08")).unwrap();
        std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();
        std::fs::create_dir_all(cogz_dir.join("knowledge/test")).unwrap();

        let db_path = cogz_dir.join("cogz.db");
        let storage = Storage::open(&db_path, 768).unwrap();
        (dir, storage)
    }

    fn make_observation(cogz_dir: &Path, id: &str, title: &str, content: &str) -> EntityFile {
        let mut entity = EntityFile::new(title, FileEntityType::Observation, content);
        entity.id = id.to_string();
        entity.status = "active".to_string();
        let path = entity.file_path(cogz_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        write_entity_file(&path, &entity).unwrap();
        entity
    }

    fn make_db_observation(id: &str, title: &str, content: &str) -> Entity {
        let year_month = chrono::Utc::now().format("%Y-%m").to_string();
        let mut entity = Entity::new(id, "observation", title, content);
        entity.file_path = Some(format!("observations/{year_month}/{id}.md"));
        entity
    }

    #[test]
    fn flags_observation_referencing_changed_code() {
        let (dir, storage) = setup();
        let cogz_dir = dir.path().join(".cogz");

        // Insert a code entity (function).
        let conn = storage.conn();
        let code_entity = Entity::new("code-uuid-1", "function", "my_func", "fn my_func() {}");
        insert_entity(&conn, &code_entity).unwrap();

        // Insert an observation that references the code entity.
        let obs = make_observation(&cogz_dir, "obs-uuid-1", "Bug in my_func", "Found a bug");
        let obs_entity = make_db_observation("obs-uuid-1", "Bug in my_func", "Found a bug");
        insert_entity(&conn, &obs_entity).unwrap();
        insert_edge(
            &conn,
            &Edge {
                source_id: "obs-uuid-1".to_string(),
                target_id: "code-uuid-1".to_string(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        drop(conn);

        // Flag stale.
        let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-1".to_string()]);
        assert_eq!(count, 1);

        // Verify DB status.
        let conn = storage.conn();
        let entity = storage::crud::get_entity(&conn, "obs-uuid-1").unwrap();
        assert_eq!(entity.status, "stale");

        // Verify file frontmatter.
        let path = obs.file_path(&cogz_dir);
        let file = read_entity_file(&path).unwrap();
        assert_eq!(file.status, "stale");

        // Verify event was recorded.
        let events = storage::events::get_recent_events(&conn, "code_changed", 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["count"], 1);
    }

    #[test]
    fn does_not_flag_already_stale() {
        let (dir, storage) = setup();
        let cogz_dir = dir.path().join(".cogz");

        let conn = storage.conn();
        let code_entity = Entity::new("code-uuid-2", "function", "func2", "fn func2() {}");
        insert_entity(&conn, &code_entity).unwrap();

        let mut obs = make_observation(&cogz_dir, "obs-uuid-2", "Note about func2", "Some note");
        obs.status = "stale".to_string();
        let path = obs.file_path(&cogz_dir);
        write_entity_file(&path, &obs).unwrap();

        let obs_entity = make_db_observation("obs-uuid-2", "Note about func2", "Some note");
        let mut db_entity = obs_entity.clone();
        db_entity.status = "stale".to_string();
        insert_entity(&conn, &db_entity).unwrap();
        insert_edge(
            &conn,
            &Edge {
                source_id: "obs-uuid-2".to_string(),
                target_id: "code-uuid-2".to_string(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        drop(conn);

        let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-2".to_string()]);
        assert_eq!(count, 0);
    }

    #[test]
    fn does_not_flag_unrelated_entities() {
        let (dir, storage) = setup();
        let cogz_dir = dir.path().join(".cogz");

        let conn = storage.conn();
        let code_entity = Entity::new("code-uuid-3", "function", "func3", "fn func3() {}");
        insert_entity(&conn, &code_entity).unwrap();

        // Observation that does NOT reference the code entity.
        make_observation(
            &cogz_dir,
            "obs-uuid-3",
            "Unrelated note",
            "Nothing about func3",
        );
        let obs_entity = make_db_observation("obs-uuid-3", "Unrelated note", "Nothing about func3");
        insert_entity(&conn, &obs_entity).unwrap();
        drop(conn);

        let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-3".to_string()]);
        assert_eq!(count, 0);
    }

    #[test]
    fn empty_changed_ids_returns_zero() {
        let (dir, storage) = setup();
        let cogz_dir = dir.path().join(".cogz");

        let count = flag_stale_knowledge(&storage, &cogz_dir, &[]);
        assert_eq!(count, 0);
    }

    #[test]
    fn flags_multiple_referencing_entities() {
        let (dir, storage) = setup();
        let cogz_dir = dir.path().join(".cogz");

        let conn = storage.conn();
        let code_entity = Entity::new(
            "code-uuid-4",
            "function",
            "shared_func",
            "fn shared_func() {}",
        );
        insert_entity(&conn, &code_entity).unwrap();

        // Two observations referencing the same code entity.
        for i in 0..2 {
            let id = format!("obs-multi-{i}");
            make_observation(&cogz_dir, &id, &format!("Note {i}"), "content");
            let entity = make_db_observation(&id, &format!("Note {i}"), "content");
            insert_entity(&conn, &entity).unwrap();
            insert_edge(
                &conn,
                &Edge {
                    source_id: id.clone(),
                    target_id: "code-uuid-4".to_string(),
                    edge_type: "references".to_string(),
                    weight: 1.0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .unwrap();
        }
        drop(conn);

        let count = flag_stale_knowledge(&storage, &cogz_dir, &["code-uuid-4".to_string()]);
        assert_eq!(count, 2);
    }
}
