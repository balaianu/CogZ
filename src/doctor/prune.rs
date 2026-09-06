//! Observation pruning with tombstones.
//!
//! Pruned entities become minimal DB records: `status='pruned'`,
//! `content=''`, no embedding, no FTS entry. Graph edges to/from
//! tombstoned entities are preserved. Active and stale observations
//! are never pruned. Rules and knowledge are never pruned.
//!
//! The canonical file is kept on disk with `status: pruned` and an
//! empty body, so `cogz reset` + `cogz index` reproduces the pruned
//! state. The file's frontmatter (including references) is preserved
//! so graph edges are rebuildable.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::files::{read_entity_file, write_entity_file};
use crate::storage::Storage;
use crate::storage::crud::{delete_entities_cascade_batch, tombstone_entity_with_timestamp};
use crate::storage::embeddings::delete_embedding;
use crate::storage::query::{
    count_by_status, find_prune_candidates as find_prune_candidate_rows, get_oldest_tombstone_ids,
};

/// A candidate for pruning.
#[derive(Debug, Clone)]
pub struct PruneCandidate {
    pub entity_id: String,
    pub file_path: String,
    pub status: String,
    pub age_days: i64,
}

/// Result of a prune operation.
#[derive(Debug, Default)]
pub struct PruneReport {
    pub candidates: Vec<PruneCandidate>,
    pub pruned: usize,
    pub tombstones_removed: usize,
    pub skipped: usize,
}

/// Find observations eligible for pruning.
///
/// Eligible: status is `rejected` or `superseded`, and age exceeds
/// `observation_prune_after_days`. Active and stale observations are
/// never prunable.
pub fn find_prune_candidates(storage: &Storage, config: &Config) -> Vec<PruneCandidate> {
    let conn = storage.conn();
    let threshold_days = config.retention.observation_prune_after_days;
    let now = Utc::now();

    let rows = match find_prune_candidate_rows(&conn) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to query prune candidates: {}", e);
            return Vec::new();
        }
    };

    let mut candidates = Vec::new();
    for row in rows {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&row.updated_at) {
            let age_days = (now - dt.with_timezone(&Utc)).num_days();
            if age_days >= threshold_days as i64 {
                candidates.push(PruneCandidate {
                    entity_id: row.entity_id,
                    file_path: row.file_path,
                    status: row.status,
                    age_days,
                });
            }
        }
    }

    candidates
}

/// Run the prune operation.
///
/// If `confirm` is false, this is a dry run — it only reports
/// candidates. If `confirm` is true, it converts observation files
/// to tombstones (status=pruned, empty body) and replaces DB entities
/// with tombstones. The file is kept on disk for rebuildability.
pub fn run_prune(
    storage: &Storage,
    config: &Config,
    cogz_dir: &Path,
    confirm: bool,
) -> PruneReport {
    let candidates = find_prune_candidates(storage, config);

    if !confirm {
        return PruneReport {
            candidates,
            ..Default::default()
        };
    }

    let mut report = PruneReport {
        candidates: candidates.clone(),
        ..Default::default()
    };

    // Phase 1: Filesystem work — convert canonical files to tombstones
    // without holding the DB mutex. Collect the IDs and updated_at
    // timestamps of successfully tombstoned files for the DB phase.
    // Hold the canonical-file write lock to prevent concurrent writers
    // from clobbering tombstone writes.
    let _file_lock = storage.file_lock();
    let mut tombstoned: Vec<(String, String)> = Vec::new();
    for candidate in &candidates {
        let file_path = cogz_dir.join(&candidate.file_path);
        match read_entity_file(&file_path) {
            Ok(mut entity_file) => {
                entity_file.status = "pruned".to_string();
                entity_file.title = String::new();
                entity_file.body = String::new();
                entity_file.updated_at = chrono::Utc::now().to_rfc3339();
                if let Err(e) = write_entity_file(&file_path, &entity_file) {
                    tracing::warn!(
                        "failed to write tombstone file {}: {} — skipping DB tombstone",
                        file_path.display(),
                        e
                    );
                    report.skipped += 1;
                    continue;
                }
                tombstoned.push((candidate.entity_id.clone(), entity_file.updated_at.clone()));
            }
            Err(e) => {
                tracing::warn!(
                    "failed to read file for tombstone {}: {} — skipping (no DB-only tombstone)",
                    file_path.display(),
                    e
                );
                report.skipped += 1;
                continue;
            }
        }
    }

    // Phase 2: DB work — tombstone entities and delete embeddings
    // under the storage lock. File I/O is already complete. Use the
    // file's updated_at so reset + index reproduces the same row.
    {
        let conn = storage.conn();
        for (entity_id, updated_at) in &tombstoned {
            if let Err(e) = tombstone_entity_with_timestamp(&conn, entity_id, updated_at) {
                tracing::warn!("failed to tombstone {}: {}", entity_id, e);
                report.skipped += 1;
                continue;
            }
            let _ = delete_embedding(&conn, entity_id);
            report.pruned += 1;
        }

        // Enforce tombstone_max_count — remove oldest tombstones.
        let max = config.retention.tombstone_max_count as usize;
        report.tombstones_removed = enforce_tombstone_limit(&conn, max, cogz_dir);
    }

    report
}

/// Remove oldest tombstones that exceed the configured limit. Uses
/// storage-layer APIs for all DB operations — no direct SQL outside
/// the storage module. Also deletes the tombstone files so they don't
/// recreate the entities on the next sync. File paths are fetched
/// under the DB lock; file deletion happens after the lock is released.
fn enforce_tombstone_limit(conn: &rusqlite::Connection, max: usize, cogz_dir: &Path) -> usize {
    let count = count_by_status(conn, "pruned").unwrap_or(0);

    tracing::debug!("enforce_tombstone_limit: count={}, max={}", count, max);

    if count <= max as i64 {
        return 0;
    }

    let excess = count - max as i64;

    let old_tombstones = match get_oldest_tombstone_ids(conn, excess) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("failed to query oldest tombstones: {}", e);
            return 0;
        }
    };

    // Fetch file paths before deleting so we can remove the tombstone files.
    let file_paths: Vec<String> = old_tombstones
        .iter()
        .filter_map(|id| {
            crate::storage::crud::get_entity(conn, id)
                .ok()
                .and_then(|e| e.file_path)
        })
        .collect();

    let mut removed = 0;
    match delete_entities_cascade_batch(conn, &old_tombstones) {
        Ok(n) => removed = n,
        Err(e) => {
            tracing::warn!("failed to batch delete tombstones: {}", e);
        }
    }

    // Delete the tombstone files for permanently removed entities.
    // This runs while the caller still holds the connection, but file
    // deletion is fast and non-blocking compared to the read/write
    // operations in the main prune loop.
    for fp in &file_paths {
        let path = cogz_dir.join(fp);
        if path.exists()
            && let Err(e) = std::fs::remove_file(&path)
        {
            tracing::warn!("failed to delete tombstone file {}: {}", path.display(), e);
        }
    }

    removed
}
