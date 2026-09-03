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
mod tests;

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage;
use crate::storage::crud::{Entity, EntityType};

/// Parsed entity ready for DB sync: (id, entity).
type ParsedEntity = (String, Entity);

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
}

/// Generate a deterministic UUID v5 for a code entity.
///
/// The name is `{file_path}:{entity_type}:{qualified_name}`. This
/// ensures the same code entity always gets the same UUID, even after
/// a full DB rebuild.
pub fn code_entity_uuid(file_path: &str, entity_type: &str, qualified_name: &str) -> String {
    let name = format!("{file_path}:{entity_type}:{qualified_name}");
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
) -> CodeSyncResult {
    let mut result = CodeSyncResult::default();

    // Phase 1: convert CodeEntity values to ParsedEntity (no lock held).
    let (all_entities, file_to_entity_ids) = convert_entities(entities_by_file);

    // Phase 2: DB operations (lock held).
    let conn = storage.conn();

    sync_entities_to_db(&conn, &all_entities, &mut result);

    // Mark stale: code entities whose file_path is not in the current
    // source file set. We query all code entities and check.
    result.marked_stale = mark_stale_code_entities(&conn, &file_to_entity_ids);

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
        .map(|p| p.to_string_lossy().to_string())
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

    // Batch-mark stale. The WHERE clause enforces active→stale,
    // which is a legal transition per the status state machine.
    let now = chrono::Utc::now().to_rfc3339();
    let mut count = 0;
    for chunk in stale_ids.chunks(999) {
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE entities SET status = 'stale', updated_at = ? \
             WHERE id IN ({placeholders}) AND status = 'active'"
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + chunk.len());
        params.push(&now);
        for id in chunk {
            params.push(id);
        }
        match conn.execute(&sql, params.as_slice()) {
            Ok(n) => count += n,
            Err(e) => tracing::warn!("failed to mark removed entities stale: {}", e),
        }
    }

    (count, stale_ids)
}

/// Core DB sync — insert or update each entity, tracking results.
/// Batch-fetches all existing entities in one query to avoid N+1.
///
/// Counts are tracked locally and only applied to `result` after
/// successful commit. If the transaction commit fails, the counts
/// are discarded — the DB rolled back, so reporting non-zero counts
/// would be misleading.
fn sync_entities_to_db(
    conn: &rusqlite::Connection,
    all_entities: &[ParsedEntity],
    result: &mut CodeSyncResult,
) {
    let ids: Vec<String> = all_entities.iter().map(|(id, _)| id.clone()).collect();
    let existing_map: HashMap<String, storage::crud::Entity> =
        storage::crud::get_entities_batch(conn, &ids)
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect();

    // Track counts locally so they can be discarded on commit failure.
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut synced_ids: Vec<String> = Vec::new();

    // Wrap all inserts/updates in a single transaction to avoid
    // one fsync per row. On a 1000+ entity codebase this reduces
    // the DB sync phase from minutes to seconds.
    let in_transaction = conn.execute_batch("BEGIN").is_ok();
    if !in_transaction {
        tracing::warn!("failed to begin entity transaction — falling back to autocommit");
    }

    for (id, entity) in all_entities {
        match existing_map.get(id) {
            Some(existing) => {
                if existing.status == "stale" {
                    // Reactivate stale entity even if content is unchanged.
                    let mut updated_entity = entity.clone();
                    updated_entity.status = "active".to_string();
                    updated_entity.created_at = existing.created_at.clone();
                    if let Err(e) = storage::crud::update_entity(conn, &updated_entity) {
                        tracing::warn!("failed to reactivate code entity {}: {}", id, e);
                        continue;
                    }
                    updated += 1;
                    synced_ids.push(id.clone());
                    continue;
                }
                if existing.content_hash == entity.content_hash {
                    skipped += 1;
                    continue;
                }
                // Update content, properties, hash. Preserve status.
                let mut updated_entity = entity.clone();
                updated_entity.status = existing.status.clone();
                updated_entity.created_at = existing.created_at.clone();
                if let Err(e) = storage::crud::update_entity(conn, &updated_entity) {
                    tracing::warn!("failed to update code entity {}: {}", id, e);
                    continue;
                }
                updated += 1;
                synced_ids.push(id.clone());
            }
            None => {
                if let Err(e) = storage::crud::insert_entity(conn, entity) {
                    tracing::warn!("failed to insert code entity {}: {}", id, e);
                    continue;
                }
                created += 1;
                synced_ids.push(id.clone());
            }
        }
    }

    if in_transaction && let Err(e) = conn.execute_batch("COMMIT") {
        tracing::warn!("failed to commit entity transaction: {}", e);
        // Transaction rolled back — discard created/updated counts
        // since those writes were undone. But skipped entities were
        // never modified (hash matched, no SQL executed), so the
        // skipped count is still accurate.
        result.skipped += skipped;
        return;
    }

    result.created += created;
    result.updated += updated;
    result.skipped += skipped;
    result.synced_entity_ids.extend(synced_ids);
}

/// Mark code entities as stale if their source file is no longer present.
///
/// Returns the number of entities marked stale. Uses a single batched
/// UPDATE query instead of fetching all entities and updating in a loop.
fn mark_stale_code_entities(
    conn: &rusqlite::Connection,
    current_files: &HashMap<String, Vec<String>>,
) -> usize {
    // Find active code entities whose file_path is NOT in current_files.
    // We need to query the file paths first, then batch-update.
    let code_types = ["function", "class", "file", "module"];
    let type_placeholders = (0..code_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT DISTINCT file_path FROM entities \
         WHERE status = 'active' AND type IN ({type_placeholders}) \
         AND file_path IS NOT NULL"
    );
    let params: Vec<&dyn rusqlite::ToSql> = code_types
        .iter()
        .map(|t| t as &dyn rusqlite::ToSql)
        .collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("failed to query code entity file paths: {}", e);
            return 0;
        }
    };
    let rows = match stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to query code entity file paths: {}", e);
            return 0;
        }
    };

    let mut stale_paths: Vec<String> = Vec::new();
    for row in rows.flatten() {
        if !current_files.contains_key(&row) {
            stale_paths.push(row);
        }
    }

    storage::crud::mark_code_entities_stale_by_file_paths(conn, &stale_paths).unwrap_or(0)
}
