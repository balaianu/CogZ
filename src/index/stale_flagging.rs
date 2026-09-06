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

/// Find entities with `references` or `auto_references` edges to any
/// of the changed code entity IDs. Returns the referencing entity IDs
/// (deduplicated).
///
/// `auto_references` edges are created by `auto_link::sync_auto_links`
/// from knowledge content scanning. They must be covered here so that
/// auto-linked knowledge is also flagged stale when its target code
/// changes — otherwise stale auto-links silently remain `active`.
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
        if (edge_type == "references" || edge_type == "auto_references")
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

    // Hold the canonical-file write lock across all read-modify-write
    // sequences to prevent concurrent knowledge updates from
    // clobbering stale-flagging writes (or vice versa).
    let _file_lock = storage.file_lock();

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

        // 2. Validate the transition.
        if let Err(e) = transition_status(&entity.status, "stale") {
            tracing::warn!("illegal status transition for {}: {}", entity.id, e);
            // The file was already written — revert it.
            entity_file.status = entity.status.clone();
            let _ = write_entity_file(path, &entity_file);
            continue;
        }

        // 3. Sync the file to the DB so that status, properties,
        //    timestamps, content_hash, and edges are all derived
        //    from the canonical file. This avoids hash drift between
        //    the file and the DB row.
        let rel_path = path.strip_prefix(cogz_dir).unwrap_or(path);
        let sync_result =
            crate::files::sync_single_file(storage, cogz_dir, &rel_path.to_string_lossy());
        if sync_result.errors.is_empty() {
            flagged_ids.push(entity.id.clone());
        } else {
            tracing::warn!(
                "failed to sync stale-flagged file {}: {}",
                path.display(),
                sync_result.errors[0].error
            );
        }
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
#[path = "stale_flagging_tests.rs"]
mod tests;
