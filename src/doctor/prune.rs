//! Observation pruning with tombstones.
//!
//! Pruned entities become minimal DB records: `status='pruned'`,
//! `content=''`, no embedding, no FTS entry. Graph edges to/from
//! tombstoned entities are preserved. Active and stale observations
//! are never pruned. Rules and knowledge are never pruned.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::storage::Storage;
use crate::storage::crud::{delete_entities_cascade_batch, tombstone_entity};
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
/// candidates. If `confirm` is true, it deletes observation files
/// and replaces DB entities with tombstones.
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

    let conn = storage.conn();

    for candidate in &candidates {
        // Tombstone the DB entity first, then delete the file. If the
        // tombstone fails, the file is still on disk and the entity is
        // still in its terminal state (rejected/superseded) — a safe
        // state that can be retried. Deleting the file first would lose
        // the content irreversibly if the tombstone fails.
        if let Err(e) = tombstone_entity(&conn, &candidate.entity_id) {
            tracing::warn!("failed to tombstone {}: {}", candidate.entity_id, e);
            report.skipped += 1;
            continue;
        }

        // Remove embedding for the tombstoned entity.
        // FTS is already cleaned by the UPDATE trigger (entities_fts_au).
        let _ = delete_embedding(&conn, &candidate.entity_id);

        // Delete the observation file now that the DB tombstone succeeded.
        let file_path = cogz_dir.join(&candidate.file_path);
        if file_path.exists()
            && let Err(e) = std::fs::remove_file(&file_path)
        {
            tracing::warn!("failed to delete {}: {}", file_path.display(), e);
            // The entity is already tombstoned — the file leak is
            // cosmetic. The next sync will mark it stale (file exists
            // but entity is pruned), but that's harmless.
        }

        report.pruned += 1;
    }

    // Enforce tombstone_max_count — remove oldest tombstones.
    let max = config.retention.tombstone_max_count as usize;
    report.tombstones_removed = enforce_tombstone_limit(&conn, max);

    report
}

/// Remove oldest tombstones that exceed the configured limit. Uses
/// storage-layer APIs for all DB operations — no direct SQL outside
/// the storage module.
fn enforce_tombstone_limit(conn: &rusqlite::Connection, max: usize) -> usize {
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

    let mut removed = 0;
    match delete_entities_cascade_batch(conn, &old_tombstones) {
        Ok(n) => removed = n,
        Err(e) => {
            tracing::warn!("failed to batch delete tombstones: {}", e);
        }
    }

    removed
}
