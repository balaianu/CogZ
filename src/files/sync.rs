//! File → database synchronization.
//!
//! Scans `.cogz/knowledge/`, `.cogz/rules/`, `.cogz/observations/`
//! for entity files, computes content hashes, and syncs the database
//! to match. Files are canonical; the DB is derived.
//!
//! Sync rules:
//! - New file → create DB entity, index in FTS
//! - Changed file (content hash differs) → update DB entity
//! - Deleted file → mark DB entity status = 'stale' (preserve edges)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::storage;
use crate::storage::crud::{ENTITY_COLUMNS, Entity, EntityType};

use super::entities::{EntityFile, FileEntityType, fm_value_to_json};

/// Subdirectories of `.cogz/` that contain entity files.
const ENTITY_DIRS: &[&str] = &["knowledge", "rules", "observations"];

/// Internal error type for sync operations.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("frontmatter error: {0}")]
    Frontmatter(#[from] super::FrontmatterError),
    #[error("storage error: {0}")]
    Storage(#[from] storage::StorageError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A file-level sync failure — associates a file path with the error.
#[derive(Debug)]
pub struct SyncFailure {
    pub file_path: PathBuf,
    pub error: SyncError,
}

/// Result of a sync operation.
#[derive(Debug, Default)]
pub struct SyncResult {
    pub created: usize,
    pub updated: usize,
    pub marked_stale: usize,
    pub skipped: usize,
    pub errors: Vec<SyncFailure>,
    /// IDs of entities that were created or updated (for embedding).
    pub synced_entity_ids: Vec<String>,
}

/// Compute SHA-256 hash of file content.
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Scan a `.cogz/` directory and return all entity file paths found.
pub fn scan_entity_files(cogz_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    for subdir in ENTITY_DIRS {
        let dir = cogz_dir.join(subdir);
        if !dir.is_dir() {
            continue;
        }

        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    files.push(path.to_path_buf());
                }
            }
        }
    }

    files
}

/// Full sync: scan all entity files and sync them to the database.
/// Also marks DB entities whose files have been deleted as stale.
pub fn sync_all(storage: &storage::Storage, cogz_dir: &Path) -> SyncResult {
    sync_inner(storage, cogz_dir, false)
}

/// Incremental sync: only process files that have changed (different
/// content hash) or are new. More efficient than full sync when most
/// files haven't changed.
pub fn sync_incremental(storage: &storage::Storage, cogz_dir: &Path) -> SyncResult {
    sync_inner(storage, cogz_dir, true)
}

/// Shared sync implementation. When `incremental` is true, files whose
/// content hash hasn't changed are skipped.
///
/// File reads happen without holding the storage mutex; DB operations
/// acquire it in a second phase. This avoids blocking other callers
/// during filesystem I/O.
fn sync_inner(storage: &storage::Storage, cogz_dir: &Path, incremental: bool) -> SyncResult {
    let mut result = SyncResult::default();

    // Phase 1: scan and parse all files (no lock held).
    let disk_files = scan_entity_files(cogz_dir);
    let mut parsed: Vec<(PathBuf, EntityFile, String, String)> = Vec::new();
    for file_path in &disk_files {
        match read_and_parse(file_path, cogz_dir) {
            Ok((ef, hash, rel_path)) => {
                parsed.push((file_path.clone(), ef, hash, rel_path));
            }
            Err(error) => result.errors.push(SyncFailure {
                file_path: file_path.clone(),
                error,
            }),
        }
    }

    // Phase 2: DB operations (lock held).
    let conn = storage.conn();
    let mut seen_ids: HashMap<String, PathBuf> = HashMap::new();
    let mut synced_files: Vec<EntityFile> = Vec::new();

    for (file_path, ef, hash, rel_path) in &parsed {
        match sync_parsed_file(&conn, ef, hash, rel_path, incremental) {
            Ok(action) => {
                let was_skipped = action == SyncAction::Skipped;
                record_action(&mut result, action, &ef.id);
                seen_ids.insert(ef.id.clone(), file_path.clone());
                // Only re-sync references for files that changed.
                // Skipped files haven't changed, so their references
                // are already correct.
                if !was_skipped {
                    synced_files.push(ef.clone());
                }
            }
            Err(error) => result.errors.push(SyncFailure {
                file_path: file_path.clone(),
                error,
            }),
        }
    }

    // Second pass: sync reference edges now that all entities exist.
    // This handles forward references — edges to entities that were
    // synced later in the same pass. Done once for all changed files.
    for ef in &synced_files {
        if let Err(e) = super::refs::sync_references(&conn, ef) {
            result.errors.push(SyncFailure {
                file_path: ef.file_path(cogz_dir),
                error: SyncError::Storage(e),
            });
        }
    }

    mark_deleted_as_stale(&conn, &seen_ids, cogz_dir, &mut result);

    // Record last index timestamp in meta table.
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = storage::set_meta(&conn, "last_index", &now) {
        tracing::warn!("failed to record last_index: {}", e);
    }

    result
}

/// Action taken for a single file during sync.
#[derive(Debug, PartialEq, Eq)]
enum SyncAction {
    Created,
    Updated,
    Skipped,
}

/// Update result counts and synced IDs based on the sync action.
fn record_action(result: &mut SyncResult, action: SyncAction, entity_id: &str) {
    match action {
        SyncAction::Created => {
            result.created += 1;
            result.synced_entity_ids.push(entity_id.to_string());
        }
        SyncAction::Updated => {
            result.updated += 1;
            result.synced_entity_ids.push(entity_id.to_string());
        }
        SyncAction::Skipped => result.skipped += 1,
    }
}

/// Read a file from disk, parse it, and compute its content hash.
/// No DB lock required — pure filesystem I/O.
fn read_and_parse(
    file_path: &Path,
    cogz_dir: &Path,
) -> Result<(EntityFile, String, String), SyncError> {
    let raw_content = std::fs::read_to_string(file_path)?;
    let entity_file = EntityFile::from_content(&raw_content)?;
    let hash = content_hash(&raw_content);
    let relative_path = file_path
        .strip_prefix(cogz_dir)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    Ok((entity_file, hash, relative_path))
}

/// Sync a parsed entity file to the DB. Requires the storage lock.
fn sync_parsed_file(
    conn: &rusqlite::Connection,
    entity_file: &EntityFile,
    hash: &str,
    relative_path: &str,
    incremental: bool,
) -> Result<SyncAction, SyncError> {
    match storage::crud::get_entity(conn, &entity_file.id) {
        Ok(existing) => {
            if incremental && existing.content_hash.as_deref() == Some(hash) {
                return Ok(SyncAction::Skipped);
            }
            let content_changed = existing.content != entity_file.body;
            let entity = build_entity(entity_file, hash, relative_path);
            update_entity_preserving_status(conn, &existing, &entity)?;
            if content_changed {
                super::events::record_edit_event(conn, entity_file, &existing.content);
            }
            Ok(SyncAction::Updated)
        }
        Err(storage::StorageError::EntityNotFound(_)) => {
            let entity = build_entity(entity_file, hash, relative_path);
            storage::crud::insert_entity(conn, &entity)?;
            super::events::record_create_event(conn, entity_file);
            Ok(SyncAction::Created)
        }
        Err(e) => Err(SyncError::Storage(e)),
    }
}

/// Update an entity, preserving the DB status if the file's status
/// would require an illegal transition. The file is canonical, but
/// status transitions must go through the state machine.
fn update_entity_preserving_status(
    conn: &rusqlite::Connection,
    existing: &Entity,
    new_entity: &Entity,
) -> Result<(), SyncError> {
    if existing.status != new_entity.status
        && storage::status::transition_status(&existing.status, &new_entity.status).is_err()
    {
        // Illegal transition — keep existing DB status, update other fields
        let mut entity = new_entity.clone();
        entity.status = existing.status.clone();
        storage::crud::update_entity(conn, &entity)?;
    } else {
        storage::crud::update_entity(conn, new_entity)?;
    }
    Ok(())
}

/// Build a storage Entity from an EntityFile.
fn build_entity(entity_file: &EntityFile, hash: &str, relative_path: &str) -> Entity {
    let entity_type = match entity_file.entity_type {
        FileEntityType::Observation => EntityType::Observation,
        FileEntityType::Rule => EntityType::Rule,
        FileEntityType::Knowledge => EntityType::Knowledge,
    };

    let mut properties = serde_json::Map::new();
    const COMMON_FIELDS: &[&str] = &[
        "id",
        "title",
        "type",
        "status",
        "created_at",
        "updated_at",
        "references",
    ];
    for (key, value) in &entity_file.frontmatter.entries {
        if !COMMON_FIELDS.contains(&key.as_str()) {
            properties.insert(key.clone(), fm_value_to_json(value));
        }
    }

    Entity {
        id: entity_file.id.clone(),
        r#type: entity_type.as_str().to_string(),
        title: Some(entity_file.title.clone()),
        content: entity_file.body.clone(),
        properties: serde_json::Value::Object(properties),
        file_path: Some(relative_path.to_string()),
        status: entity_file.status.clone(),
        content_hash: Some(hash.to_string()),
        created_at: entity_file.created_at.clone(),
        updated_at: entity_file.updated_at.clone(),
    }
}

/// Mark DB entities as stale if their file_path no longer exists on disk.
fn mark_deleted_as_stale(
    conn: &rusqlite::Connection,
    seen_ids: &HashMap<String, PathBuf>,
    cogz_dir: &Path,
    result: &mut SyncResult,
) {
    let file_backed = match get_file_backed_entities(conn) {
        Ok(e) => e,
        Err(error) => {
            result.errors.push(SyncFailure {
                file_path: cogz_dir.to_path_buf(),
                error,
            });
            return;
        }
    };

    for entity in file_backed {
        if !seen_ids.contains_key(&entity.id) && entity.status == "active" {
            match storage::crud::update_status(conn, &entity.id, "stale") {
                Ok(()) => result.marked_stale += 1,
                Err(error) => result.errors.push(SyncFailure {
                    file_path: PathBuf::from(entity.file_path.unwrap_or_default()),
                    error: SyncError::Storage(error),
                }),
            }
        }
    }
}

/// Get all file-backed entities (those with a non-null file_path).
fn get_file_backed_entities(conn: &rusqlite::Connection) -> Result<Vec<Entity>, SyncError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ENTITY_COLUMNS} FROM entities WHERE file_path IS NOT NULL"
        ))
        .map_err(|e| SyncError::Storage(e.into()))?;
    let rows = stmt
        .query_map([], storage::crud::row_to_entity)
        .map_err(|e| SyncError::Storage(e.into()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| SyncError::Storage(e.into()))
}
