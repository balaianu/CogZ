//! File → database synchronization.
//!
//! Scans `.cogz/knowledge/`, `.cogz/rules/`, `.cogz/observations/`
//! for entity files, computes content hashes, and syncs the database
//! to match. Files are canonical; the DB is derived.
//!
//! Sync rules (from architecture.md):
//! - New file → create DB entity, index in FTS
//! - Changed file (content hash differs) → update DB entity
//! - Deleted file → mark DB entity status = 'stale' (preserve edges)
//!
//! No embeddings are generated in this layer — that's the embed
//! module's job (Phase 4). FTS5 is populated automatically by the
//! schema triggers on insert/update/delete.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::storage;
use crate::storage::crud::{Entity, EntityType};
use crate::storage::edges::Edge;

use super::entities::{EntityFile, FileEntityType, read_entity_file};
use super::frontmatter::FmValue;

/// Subdirectories of `.cogz/` that contain entity files.
const ENTITY_DIRS: &[&str] = &["knowledge", "rules", "observations"];

/// Result of a sync operation.
#[derive(Debug, Default)]
pub struct SyncResult {
    pub created: usize,
    pub updated: usize,
    pub marked_stale: usize,
    pub skipped: usize,
    pub errors: Vec<SyncError>,
}

#[derive(Debug)]
pub struct SyncError {
    pub file_path: PathBuf,
    pub message: String,
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
    let mut result = SyncResult::default();
    let conn = storage.conn();

    // Collect all file paths from disk
    let disk_files = scan_entity_files(cogz_dir);

    // Track which entity IDs we've seen on disk
    let mut seen_ids: HashMap<String, PathBuf> = HashMap::new();

    for file_path in &disk_files {
        match sync_one_file(&conn, file_path, cogz_dir) {
            Ok(SyncAction::Created) => result.created += 1,
            Ok(SyncAction::Updated) => result.updated += 1,
            Ok(SyncAction::Skipped) => result.skipped += 1,
            Err(message) => result.errors.push(SyncError {
                file_path: file_path.clone(),
                message,
            }),
        }

        // Track seen IDs (read the file again only if sync_one didn't error)
        if let Ok(entity_file) = read_entity_file(file_path) {
            seen_ids.insert(entity_file.id, file_path.clone());
        }
    }

    // Mark stale: find DB entities with file_path that no longer exist on disk
    mark_deleted_as_stale(&conn, &seen_ids, cogz_dir, &mut result);

    result
}

/// Incremental sync: only process files that have changed (different
/// content hash) or are new. More efficient than full sync when most
/// files haven't changed.
pub fn sync_incremental(storage: &storage::Storage, cogz_dir: &Path) -> SyncResult {
    let mut result = SyncResult::default();
    let conn = storage.conn();

    let disk_files = scan_entity_files(cogz_dir);
    let mut seen_ids: HashMap<String, PathBuf> = HashMap::new();

    for file_path in &disk_files {
        match sync_one_file_incremental(&conn, file_path, cogz_dir) {
            Ok(SyncAction::Created) => result.created += 1,
            Ok(SyncAction::Updated) => result.updated += 1,
            Ok(SyncAction::Skipped) => result.skipped += 1,
            Err(message) => result.errors.push(SyncError {
                file_path: file_path.clone(),
                message,
            }),
        }

        if let Ok(entity_file) = read_entity_file(file_path) {
            seen_ids.insert(entity_file.id, file_path.clone());
        }
    }

    mark_deleted_as_stale(&conn, &seen_ids, cogz_dir, &mut result);

    result
}

/// Action taken for a single file during sync.
#[derive(Debug, PartialEq, Eq)]
enum SyncAction {
    Created,
    Updated,
    Skipped,
}

/// Sync a single file to the DB. Always processes the file (full sync).
fn sync_one_file(
    conn: &rusqlite::Connection,
    file_path: &Path,
    cogz_dir: &Path,
) -> Result<SyncAction, String> {
    sync_one_file_inner(conn, file_path, cogz_dir, false)
}

/// Sync a single file, skipping if the content hash hasn't changed.
fn sync_one_file_incremental(
    conn: &rusqlite::Connection,
    file_path: &Path,
    cogz_dir: &Path,
) -> Result<SyncAction, String> {
    sync_one_file_inner(conn, file_path, cogz_dir, true)
}

/// Inner sync logic shared by full and incremental sync.
fn sync_one_file_inner(
    conn: &rusqlite::Connection,
    file_path: &Path,
    cogz_dir: &Path,
    incremental: bool,
) -> Result<SyncAction, String> {
    let entity_file = read_entity_file(file_path).map_err(|e| e.to_string())?;
    let raw_content = std::fs::read_to_string(file_path).map_err(|e| e.to_string())?;
    let hash = content_hash(&raw_content);
    let relative_path = file_path
        .strip_prefix(cogz_dir)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    match storage::crud::get_entity(conn, &entity_file.id) {
        Ok(existing) => {
            if incremental && existing.content_hash.as_deref() == Some(&hash) {
                return Ok(SyncAction::Skipped);
            }
            let content_changed = existing.content != entity_file.body;
            let entity = build_entity(&entity_file, &hash, &relative_path);
            let action = update_entity_preserving_status(conn, &existing, &entity, &entity_file)?;
            if content_changed {
                super::events::record_edit_event(conn, &entity_file, &existing.content);
            }
            Ok(action)
        }
        Err(storage::StorageError::EntityNotFound(_)) => {
            let entity = build_entity(&entity_file, &hash, &relative_path);
            storage::crud::insert_entity(conn, &entity).map_err(|e| e.to_string())?;
            sync_references(conn, &entity_file).map_err(|e| e.to_string())?;
            super::events::record_create_event(conn, &entity_file);
            Ok(SyncAction::Created)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Update an entity, preserving the DB status if the file's status
/// would require an illegal transition. The file is canonical, but
/// status transitions must go through the state machine.
fn update_entity_preserving_status(
    conn: &rusqlite::Connection,
    existing: &Entity,
    new_entity: &Entity,
    entity_file: &EntityFile,
) -> Result<SyncAction, String> {
    if existing.status != new_entity.status {
        if storage::status::transition_status(&existing.status, &new_entity.status).is_err() {
            // Illegal transition — keep existing DB status, update other fields
            let mut entity = new_entity.clone();
            entity.status = existing.status.clone();
            storage::crud::update_entity(conn, &entity).map_err(|e| e.to_string())?;
        } else {
            storage::crud::update_entity(conn, new_entity).map_err(|e| e.to_string())?;
        }
    } else {
        storage::crud::update_entity(conn, new_entity).map_err(|e| e.to_string())?;
    }
    sync_references(conn, entity_file).map_err(|e| e.to_string())?;
    Ok(SyncAction::Updated)
}

/// Build a storage Entity from an EntityFile.
fn build_entity(entity_file: &EntityFile, hash: &str, relative_path: &str) -> Entity {
    let entity_type = match entity_file.entity_type {
        FileEntityType::Observation => EntityType::Observation,
        FileEntityType::Rule => EntityType::Rule,
        FileEntityType::Knowledge => EntityType::Knowledge,
    };

    // Build properties JSON from type-specific frontmatter fields
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

/// Convert a frontmatter value to a JSON value.
fn fm_value_to_json(value: &FmValue) -> serde_json::Value {
    match value {
        FmValue::String(s) => serde_json::Value::String(s.clone()),
        FmValue::Float(f) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap_or(0.into()))
        }
        FmValue::Int(i) => serde_json::Value::Number((*i).into()),
        FmValue::Bool(b) => serde_json::Value::Bool(*b),
        FmValue::Array(a) => serde_json::Value::Array(
            a.iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    }
}

/// Sync `references` edges from the entity file to the DB.
/// Replaces all existing `references` edges for this entity.
fn sync_references(
    conn: &rusqlite::Connection,
    entity_file: &EntityFile,
) -> Result<(), rusqlite::Error> {
    // Delete existing references edges from this entity
    conn.execute(
        "DELETE FROM edges WHERE source_id = ?1 AND edge_type = 'references'",
        rusqlite::params![entity_file.id],
    )?;

    // Insert new references edges
    let now = chrono::Utc::now().to_rfc3339();
    for ref_id in &entity_file.references {
        let edge = Edge {
            source_id: entity_file.id.clone(),
            target_id: ref_id.clone(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: now.clone(),
        };
        // Use INSERT OR IGNORE — the target entity might not exist yet
        // (forward reference). We still create the edge; FK enforcement
        // would reject it, so we temporarily disable FKs for this insert.
        // Actually, the edges table has REFERENCES entities(id), so
        // inserting an edge to a non-existent entity will fail with
        // FK enforcement on. We need to handle this gracefully.
        let result = conn.execute(
            "INSERT OR IGNORE INTO edges (source_id, target_id, edge_type, weight, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                edge.source_id,
                edge.target_id,
                edge.edge_type,
                edge.weight,
                edge.created_at,
            ],
        );
        // Ignore FK errors — the referenced entity might not be synced yet
        if let Err(rusqlite::Error::SqliteFailure(err, _)) = result
            && err.code != rusqlite::ffi::ErrorCode::ConstraintViolation
        {
            return Err(rusqlite::Error::SqliteFailure(err, None));
        }
    }

    Ok(())
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
        Err(e) => {
            result.errors.push(SyncError {
                file_path: cogz_dir.to_path_buf(),
                message: format!("failed to query file-backed entities: {}", e),
            });
            return;
        }
    };

    for entity in file_backed {
        if !seen_ids.contains_key(&entity.id) && entity.status == "active" {
            match storage::crud::update_status(conn, &entity.id, "stale") {
                Ok(()) => result.marked_stale += 1,
                Err(e) => result.errors.push(SyncError {
                    file_path: PathBuf::from(entity.file_path.unwrap_or_default()),
                    message: format!("failed to mark stale: {}", e),
                }),
            }
        }
    }
}

/// Get all file-backed entities (those with a non-null file_path).
fn get_file_backed_entities(conn: &rusqlite::Connection) -> Result<Vec<Entity>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, type, title, content, properties, file_path, status, \
             content_hash, created_at, updated_at \
             FROM entities WHERE file_path IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], storage::crud::row_to_entity)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
