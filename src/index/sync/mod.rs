//! Code entity synchronization — insert/update/stale-mark code entities.
//!
//! Code entities (function, class, file, module) are not file-backed.
//! They are extracted from source code by tree-sitter and exist only
//! in the DB. Their UUIDs are deterministic (UUID v5) so that
//! `cogz reset` + `cogz index` produces identical IDs, preserving
//! `references` edges from observations.
//!
//! Sync rules mirror file sync:
//! - New entity → insert into DB, index in FTS
//! - Changed entity (content hash differs) → update DB entity
//! - Deleted source file → mark all its code entities as stale

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

mod db_sync;

pub(crate) use db_sync::{mark_stale_code_entities, sync_entities_to_db};

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage;
use crate::storage::crud::{Entity, EntityType};

/// Parsed entity ready for DB sync: (id, entity).
pub(crate) type ParsedEntity = (String, Entity);

/// UUID v5 namespace for CogZ code entities. Deterministic across
/// rebuilds — the same source file + entity name always maps to the
/// same UUID.
const CODE_ENTITY_NAMESPACE: Uuid = Uuid::from_bytes([
    0xc0, 0x9e, 0x4a, 0x7b, 0x1f, 0x2d, 0x4b, 0x8c, 0xa3, 0x56, 0xd1, 0xe4, 0xf2, 0x8a, 0x9c, 0x6b,
]);

/// Result of a code entity sync operation.
#[derive(Debug, Default)]
pub struct CodeSyncResult {
    pub created: usize,
    pub updated: usize,
    pub marked_stale: usize,
    pub skipped: usize,
    /// IDs of entities that were created or updated (for embedding).
    pub synced_entity_ids: Vec<String>,
    /// IDs of entities marked stale because they're no longer present
    /// in the new parse (renamed or removed within a changed file).
    pub removed_entity_ids: Vec<String>,
    /// Number of source files that failed to read or parse. Callers
    /// should not advance the incremental baseline when this is non-zero,
    /// so the next reindex retries the failed files.
    pub failed_files: usize,
}

/// Generate a deterministic UUID v5 for a code entity.
///
/// The name is `{file_path}:{entity_type}:{qualified_name}`. This
/// ensures the same code entity always gets the same UUID, even after
/// a full DB rebuild. Path separators are normalized to forward
/// slashes so the same source file produces the same UUID on all
/// platforms (Linux, macOS, Windows).
pub fn code_entity_uuid(file_path: &str, entity_type: &str, qualified_name: &str) -> String {
    let normalized = crate::index::normalize_path(file_path);
    let name = format!("{normalized}:{entity_type}:{qualified_name}");
    Uuid::new_v5(&CODE_ENTITY_NAMESPACE, name.as_bytes()).to_string()
}

/// Compute SHA-256 hash of entity content.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Sync code entities from a set of source files to the database.
///
/// `source_files` is a list of (relative_path, source_code, language)
/// tuples. The function parses each file, extracts entities, and
/// syncs them to the DB. Entities whose source file is no longer in
/// the list are marked stale.
///
/// File reads and parsing happen without holding the storage mutex;
/// DB operations acquire it in a second phase.
pub fn sync_code_entities(
    storage: &storage::Storage,
    entities_by_file: &[(String, Vec<crate::index::tree_sitter::CodeEntity>)],
    failed_paths: &std::collections::HashSet<String>,
) -> CodeSyncResult {
    let mut result = CodeSyncResult::default();

    // Phase 1: convert CodeEntity values to ParsedEntity (no lock held).
    let (all_entities, file_to_entity_ids) = convert_entities(entities_by_file);

    // Phase 2: DB operations (lock held).
    let conn = storage.conn();

    sync_entities_to_db(&conn, &all_entities, &mut result);

    // Mark stale: code entities whose file_path is not in the current
    // source file set AND not in the failed_paths set. Files that
    // failed to read are not deleted — their entities must not be
    // marked stale just because of an I/O error.
    result.marked_stale = mark_stale_code_entities(&conn, &file_to_entity_ids, failed_paths);

    // Record last code index timestamp.
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = storage::set_meta(&conn, "last_code_index", &now) {
        tracing::warn!("failed to record last_code_index: {}", e);
    }

    result
}

/// Incremental sync — same as `sync_code_entities` but skips the
/// full stale-marking sweep. Used by `reindex_code` when git diff
/// provides the exact set of changed files. Deleted files are
/// handled separately by the caller via `mark_stale_for_deleted_files`.
///
/// Per-file stale marking: for each changed file, any active code
/// entity in the DB whose ID is not in the new parse is marked stale.
/// This catches renamed or removed entities within files that still
/// exist on disk.
pub fn sync_code_entities_incremental(
    storage: &storage::Storage,
    entities_by_file: &[(String, Vec<crate::index::tree_sitter::CodeEntity>)],
) -> CodeSyncResult {
    let mut result = CodeSyncResult::default();

    let (all_entities, file_to_entity_ids) = convert_entities(entities_by_file);

    let conn = storage.conn();
    sync_entities_to_db(&conn, &all_entities, &mut result);

    // Per-file stale marking: mark entities no longer present in the
    // new parse as stale (renamed or removed within changed files).
    let (stale_count, stale_ids) = mark_stale_for_removed_entities(&conn, &file_to_entity_ids);
    result.marked_stale += stale_count;
    result.removed_entity_ids = stale_ids;

    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = storage::set_meta(&conn, "last_code_index", &now) {
        tracing::warn!("failed to record last_code_index: {}", e);
    }

    result
}

/// Convert pre-parsed CodeEntity values into ParsedEntity values
/// ready for DB sync. No lock held — pure CPU work.
fn convert_entities(
    entities_by_file: &[(String, Vec<crate::index::tree_sitter::CodeEntity>)],
) -> (Vec<ParsedEntity>, HashMap<String, Vec<String>>) {
    let mut all_entities: Vec<ParsedEntity> = Vec::new();
    let mut file_to_entity_ids: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (file_path_str, entities) in entities_by_file {
        for ce in entities {
            // File entities use the full path as qualified_name so that
            // code_graph can compute the same UUID from the path alone.
            // Other entity types use qualified_name/module_path/title.
            let qualified_name = if ce.entity_type == "file" {
                file_path_str.clone()
            } else {
                ce.properties
                    .get("qualified_name")
                    .and_then(|v| v.as_str())
                    .or_else(|| ce.properties.get("module_path").and_then(|v| v.as_str()))
                    .unwrap_or(&ce.title)
                    .to_string()
            };

            let id = code_entity_uuid(file_path_str, ce.entity_type, &qualified_name);

            // Multiple impl blocks for the same type produce the same
            // UUID. Keep the first; methods from all blocks are still
            // extracted as separate function entities.
            if !seen_ids.insert(id.clone()) {
                continue;
            }

            let hash = content_hash(&ce.content);

            let entity_type = match ce.entity_type {
                "function" => EntityType::Function,
                "class" => EntityType::Class,
                "file" => EntityType::File,
                "module" => EntityType::Module,
                _ => continue,
            };

            let now = chrono::Utc::now().to_rfc3339();
            let entity = Entity {
                id: id.clone(),
                r#type: entity_type.as_str().to_string(),
                title: Some(ce.title.clone()),
                content: ce.content.clone(),
                properties: ce.properties.clone(),
                file_path: Some(file_path_str.clone()),
                status: "active".to_string(),
                content_hash: Some(hash),
                created_at: now.clone(),
                updated_at: now,
            };

            file_to_entity_ids
                .entry(file_path_str.clone())
                .or_default()
                .push(id.clone());
            all_entities.push((id.clone(), entity));
        }
    }

    (all_entities, file_to_entity_ids)
}

/// Mark code entities as stale for a set of deleted file paths.
/// Returns the count of entities marked stale. Uses a single batched
/// UPDATE query instead of fetching all entities and updating in a loop.
pub fn mark_stale_for_deleted_files(
    storage: &storage::Storage,
    deleted_paths: &[std::path::PathBuf],
) -> usize {
    let conn = storage.conn();
    let deleted: Vec<String> = deleted_paths
        .iter()
        .map(|p| crate::index::path_to_string(p))
        .collect();

    storage::crud::mark_code_entities_stale_by_file_paths(&conn, &deleted).unwrap_or(0)
}

/// Mark code entities as stale when they exist in the DB for a changed
/// file but are no longer present in the new parse. This catches
/// renamed or removed entities within files that still exist on disk
/// (deleted files are handled by `mark_stale_for_deleted_files`).
///
/// Returns `(count, stale_entity_ids)` so callers can pass the IDs to
/// `flag_stale_knowledge` for downstream knowledge flagging.
fn mark_stale_for_removed_entities(
    conn: &rusqlite::Connection,
    file_to_entity_ids: &HashMap<String, Vec<String>>,
) -> (usize, Vec<String>) {
    if file_to_entity_ids.is_empty() {
        return (0, Vec::new());
    }

    let new_ids: std::collections::HashSet<String> = file_to_entity_ids
        .values()
        .flat_map(|ids| ids.iter().cloned())
        .collect();

    let code_types = ["function", "class", "file", "module"];
    let type_placeholders = (0..code_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    let file_paths: Vec<String> = file_to_entity_ids.keys().cloned().collect();
    let mut stale_ids: Vec<String> = Vec::new();

    // 4 type params + up to 995 file paths = 999 (SQLite variable limit).
    const CHUNK_SIZE: usize = 995;
    for chunk in file_paths.chunks(CHUNK_SIZE) {
        let path_placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id FROM entities \
             WHERE status = 'active' AND type IN ({type_placeholders}) \
             AND file_path IN ({path_placeholders})"
        );
        let mut params: Vec<&dyn rusqlite::ToSql> =
            Vec::with_capacity(code_types.len() + chunk.len());
        for t in &code_types {
            params.push(t);
        }
        for p in chunk {
            params.push(p);
        }
        if let Ok(mut stmt) = conn.prepare(&sql)
            && let Ok(rows) = stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0))
        {
            for row in rows.flatten() {
                if !new_ids.contains(&row) {
                    stale_ids.push(row);
                }
            }
        }
    }

    if stale_ids.is_empty() {
        return (0, Vec::new());
    }

    // Batch-mark stale via the storage layer (no direct SQL outside storage/).
    let count = storage::crud::mark_entities_stale_by_ids(conn, &stale_ids).unwrap_or_else(|e| {
        tracing::warn!("failed to mark removed entities stale: {}", e);
        0
    });

    (count, stale_ids)
}
