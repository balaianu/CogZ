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
//!
//! Deleted-file stale entities are a transitional DB state. After
//! `cogz reset`, they are gone — no canonical file exists to recreate
//! them. This is intentional: the file was deleted, so the entity
//! should not persist. References to the deleted entity in surviving
//! files' frontmatter are the historical record. Graph expansion
//! skips dangling references gracefully.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

#[path = "sync_ops.rs"]
mod sync_ops;

pub(crate) use sync_ops::{mark_deleted_as_stale, read_and_parse, sync_parsed_file};

use crate::storage;

use super::entities::EntityFile;

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

/// Sync a single entity file to the DB. Used by the file_save hook
/// to avoid scanning the entire `.cogz/` directory when only one file
/// changed. The file path is relative to the repo root (e.g.
/// `.cogz/knowledge/foo.md`).
pub fn sync_single_file(
    storage: &storage::Storage,
    cogz_dir: &Path,
    file_path: &str,
) -> SyncResult {
    let abs_path = if file_path.starts_with(".cogz/") || file_path.starts_with("./.cogz/") {
        // Relative to repo root — resolve via cogz_dir's parent.
        let repo_root = cogz_dir.parent().unwrap_or(cogz_dir);
        repo_root.join(file_path)
    } else {
        cogz_dir.join(file_path)
    };

    if !abs_path.exists() {
        return SyncResult::default();
    }

    // Defense-in-depth: verify the resolved path is inside cogz_dir.
    // Inputs come from hook scripts and the sync layer (controlled
    // sources), but canonicalization prevents path traversal via
    // ../ sequences in file_path.
    if let (Ok(canonical_abs), Ok(canonical_cogz)) =
        (abs_path.canonicalize(), cogz_dir.canonicalize())
        && !canonical_abs.starts_with(&canonical_cogz)
    {
        tracing::warn!(
            "refusing to sync file outside .cogz/: {} (resolved to {})",
            file_path,
            canonical_abs.display()
        );
        return SyncResult::default();
    }

    let mut result = SyncResult::default();

    // Phase 1: read and parse the file (no lock held).
    let (entity_file, hash, relative_path) = match read_and_parse(&abs_path, cogz_dir) {
        Ok(parsed) => parsed,
        Err(error) => {
            result.errors.push(SyncFailure {
                file_path: abs_path,
                error,
            });
            return result;
        }
    };

    // Phase 2: DB operations (lock held).
    {
        let conn = storage.conn();
        match sync_parsed_file(&conn, &entity_file, &hash, &relative_path, true) {
            Ok(action) => {
                let was_skipped = action == SyncAction::Skipped;
                record_action(&mut result, action, &entity_file.id);
                // Sync reference edges for changed files. Skipped files
                // haven't changed, so their references are already correct.
                if !was_skipped && let Err(e) = super::refs::sync_references(&conn, &entity_file) {
                    result.errors.push(SyncFailure {
                        file_path: abs_path.clone(),
                        error: SyncError::Storage(e),
                    });
                }
            }
            Err(error) => {
                result.errors.push(SyncFailure {
                    file_path: abs_path,
                    error,
                });
            }
        }
    }

    result
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
    // Track all paths that exist on disk, independently of parse
    // success. A malformed file is NOT a deletion — only a missing
    // file is. Using the disk file set (not the parsed set) for
    // stale marking prevents false lifecycle transitions from I/O
    // or parse errors.
    let disk_paths: std::collections::HashSet<PathBuf> = disk_files.iter().cloned().collect();
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

    // Phase 2: DB operations (lock held, single transaction).
    // Wrapping entity + reference sync in one transaction avoids
    // partially synchronized state if a later write fails, and reduces
    // commit overhead. Canonical files remain on disk for retry.
    let conn = storage.conn();
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!("failed to begin sync transaction: {e}");
            result.errors.push(SyncFailure {
                file_path: cogz_dir.to_path_buf(),
                error: SyncError::Storage(e.into()),
            });
            return result;
        }
    };

    let mut seen_ids: HashMap<String, PathBuf> = HashMap::new();
    // All successfully parsed entity files — used for the reference
    // sync pass. We include ALL entities (not just changed ones) so
    // that forward references from unchanged entities to newly created
    // entities are resolved. sync_references is idempotent (delete +
    // re-insert), so re-syncing unchanged entities is a no-op for
    // entities with no references.
    let mut all_parsed_files: Vec<EntityFile> = Vec::new();

    for (file_path, ef, hash, rel_path) in &parsed {
        match sync_parsed_file(&tx, ef, hash, rel_path, incremental) {
            Ok(action) => {
                record_action(&mut result, action, &ef.id);
                seen_ids.insert(ef.id.clone(), file_path.clone());
                all_parsed_files.push(ef.clone());
            }
            Err(error) => result.errors.push(SyncFailure {
                file_path: file_path.clone(),
                error,
            }),
        }
    }

    // Second pass: sync reference edges now that all entities exist.
    // This handles forward references — edges to entities that were
    // synced later in the same pass. Re-syncing ALL entities (not just
    // changed ones) ensures that an unchanged entity referencing a
    // newly created entity gets its edge created.
    for ef in &all_parsed_files {
        if let Err(e) = super::refs::sync_references(&tx, ef) {
            result.errors.push(SyncFailure {
                file_path: ef.file_path(cogz_dir),
                error: SyncError::Storage(e),
            });
        }
    }

    mark_deleted_as_stale(&tx, &disk_paths, cogz_dir, &mut result);

    // Record last index timestamp in meta table.
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = storage::set_meta(&tx, "last_index", &now) {
        tracing::warn!("failed to record last_index: {}", e);
    }

    if let Err(e) = tx.commit() {
        tracing::warn!("failed to commit sync transaction: {e}");
        result.errors.push(SyncFailure {
            file_path: cogz_dir.to_path_buf(),
            error: SyncError::Storage(e.into()),
        });
    }

    result
}

/// Action taken for a single file during sync.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SyncAction {
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
