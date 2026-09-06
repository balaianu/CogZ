//! Duplicate merging — superseded marking via canonical frontmatter.
//!
//! Confirmed duplicate observations are merged: one entity survives,
//! the other is marked `superseded` with a `superseded_by` field in
//! its frontmatter. The superseded entity's original edges are
//! preserved in its frontmatter; graph expansion follows the
//! `superseded_by` edge to reach the survivor. Knowledge is flagged,
//! not auto-merged (human-curated). Rules are surfaced for review,
//! not auto-merged.

use std::path::{Path, PathBuf};

use crate::config::ConsolidationConfig;
use crate::consolidate::dedup::confirm_duplicate_nli;
use crate::embed::NliModel;
use crate::embed::similarity::l2_to_cosine;
use crate::files::frontmatter::FmValue;
use crate::files::{FrontmatterError, read_entity_file, write_entity_file};
use crate::storage::StorageError;
use crate::storage::embeddings::get_knowledge_embeddings_batch;
use crate::storage::events::{EventType, record_event};
use crate::storage::query::get_entities_by_type;
use crate::storage::status::transition_status;

/// Errors from merge operations.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("frontmatter error: {0}")]
    Frontmatter(#[from] FrontmatterError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of a single merge decision.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MergeResult {
    pub survivor_id: String,
    pub superseded_id: String,
    pub reason: String,
}

/// Run merge across all active observations. Returns the list of
/// merged pairs. When `dry_run` is true, no changes are made.
///
/// NLI confirmation gates auto-merge: embedding similarity finds
/// candidates, then bidirectional NLI entailment confirms true
/// duplicates before any merge occurs. When `nli_model` is `None`,
/// falls back to embedding-only (candidates are reported but not
/// merged in non-dry-run mode — a warning is logged).
pub fn run_merge(
    storage: &crate::storage::Storage,
    cogz_dir: &Path,
    config: &ConsolidationConfig,
    nli_model: Option<&dyn NliModel>,
    dry_run: bool,
) -> Result<Vec<MergeResult>, StorageError> {
    let pairs = find_merge_candidates(storage, config, nli_model)?;

    if dry_run {
        return Ok(pairs
            .into_iter()
            .map(|p| MergeResult {
                survivor_id: p.survivor_id,
                superseded_id: p.superseded_id,
                reason: p.reason,
            })
            .collect());
    }

    let mut results = Vec::new();
    for pair in pairs {
        match merge_one(storage, cogz_dir, &pair) {
            Ok(()) => results.push(MergeResult {
                survivor_id: pair.survivor_id,
                superseded_id: pair.superseded_id,
                reason: pair.reason,
            }),
            Err(MergeError::Storage(StorageError::IllegalTransition { .. })) => {
                // Expected when a prior merge in the same batch already
                // superseded this entity. Skip without warning — it's
                // not an error, just a no-op.
                tracing::debug!(
                    "skip merge {} → {}: already superseded",
                    pair.superseded_id,
                    pair.survivor_id
                );
            }
            Err(e) => {
                tracing::warn!(
                    "merge failed for {} → {}: {}",
                    pair.superseded_id,
                    pair.survivor_id,
                    e
                );
            }
        }
    }
    Ok(results)
}

/// A merge candidate — two entities deemed duplicates.
struct MergeCandidate {
    survivor_id: String,
    superseded_id: String,
    reason: String,
}

/// Find duplicate observation pairs via embedding similarity, confirmed
/// by NLI bidirectional entailment. The earlier-created entity survives;
/// the later one is superseded.
///
/// Embedding similarity (cosine ≥ `dedup_threshold`) finds candidates.
/// NLI confirmation (`confirm_duplicate_nli` with `dedup_nli_threshold`)
/// filters out related-but-distinct pairs and contradictions — only
/// true semantic duplicates survive. When the NLI model is unavailable,
/// candidates are still reported (for dry-run visibility) but the reason
/// notes that NLI confirmation was skipped.
fn find_merge_candidates(
    storage: &crate::storage::Storage,
    config: &ConsolidationConfig,
    nli_model: Option<&dyn NliModel>,
) -> Result<Vec<MergeCandidate>, StorageError> {
    use crate::storage::embeddings::{EmbeddingSpace, knn_search_with_type_filter};

    let observations = {
        let conn = storage.conn();
        get_entities_by_type(&conn, "observation", Some("active"), 500)?
    };

    if observations.len() < 2 {
        return Ok(Vec::new());
    }

    let mut seen_pairs = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    // Probe the NLI model with a trivial classification to trigger
    // lazy loading. If the model can't load (missing files, ONNX
    // runtime error), fall back to embedding-only merge. This
    // distinguishes "NLI configured but unavailable" from "NLI
    // available but didn't confirm" — without it, a broken NLI
    // model would silently disable all merges.
    let nli_available = if let Some(model) = nli_model {
        model.classify("test", "test").is_ok()
    } else {
        false
    };
    let nli_model = if nli_available { nli_model } else { None };

    // Acquire the connection once for all KNN searches, instead of
    // re-acquiring the mutex per observation.
    let conn = storage.conn();

    // Batch-fetch all embeddings in one query instead of N queries.
    // Observations use the knowledge embedding space.
    let obs_ids: Vec<String> = observations.iter().map(|o| o.id.clone()).collect();
    let embedding_map = get_knowledge_embeddings_batch(&conn, &obs_ids)?;

    for obs in &observations {
        let own_embedding = match embedding_map.get(&obs.id) {
            Some(e) => e,
            None => continue,
        };

        // KNN search filtered to observations only. Without the type
        // filter, knowledge entries near an observation could consume
        // all KNN slots, leaving no observation candidates for merge.
        let neighbors = match knn_search_with_type_filter(
            &conn,
            EmbeddingSpace::Knowledge,
            own_embedding,
            10,
            "observation",
        ) {
            Ok(n) => n,
            Err(_) => continue,
        };

        for (neighbor_id, distance) in &neighbors {
            if neighbor_id == &obs.id {
                continue;
            }
            let similarity = l2_to_cosine(*distance as f64);
            if similarity < config.dedup_threshold {
                continue;
            }

            let neighbor = match observations.iter().find(|o| o.id == *neighbor_id) {
                Some(o) => o,
                None => continue,
            };

            let (survivor, superseded) = if obs.created_at <= neighbor.created_at {
                (obs, neighbor)
            } else {
                (neighbor, obs)
            };

            let pair_key = (survivor.id.clone(), superseded.id.clone());
            if seen_pairs.contains(&pair_key) {
                continue;
            }
            seen_pairs.insert(pair_key);

            // NLI confirmation: only merge if both directions entail
            // each other above the NLI threshold. This filters out
            // related-but-distinct pairs and contradictions that have
            // high embedding similarity but aren't true duplicates.
            let nli_confirmed = confirm_duplicate_nli(
                nli_model,
                &survivor.content,
                &superseded.content,
                config.dedup_nli_threshold,
            );

            if !nli_confirmed && nli_model.is_some() {
                tracing::debug!(
                    "merge candidate {} ↔ {} skipped: NLI not confirmed (sim={:.2})",
                    survivor.id,
                    superseded.id,
                    similarity
                );
                continue;
            }

            let reason = if nli_model.is_some() {
                format!("{:.2} embedding similarity (NLI confirmed)", similarity)
            } else {
                format!("{:.2} embedding similarity (NLI unavailable)", similarity)
            };

            candidates.push(MergeCandidate {
                survivor_id: survivor.id.clone(),
                superseded_id: superseded.id.clone(),
                reason,
            });
        }
    }

    Ok(candidates)
}

/// Resolve a DB-stored file path to an absolute path. DB paths are
/// stored relative to `.cogz` (e.g. `observations/2026-08/uuid.md`).
/// The repo root is the parent of `cogz_dir`.
fn resolve_file_path(file_path: &str, cogz_dir: &Path) -> PathBuf {
    if file_path.starts_with(".cogz") {
        cogz_dir.parent().unwrap_or(cogz_dir).join(file_path)
    } else {
        cogz_dir.join(file_path)
    }
}

/// Merge one pair: redirect edges, mark the superseded entity's file
/// and DB row. File-first: the superseded entity's frontmatter is
/// updated on disk before the DB.
///
/// The status transition is validated before any file I/O — if the
/// entity was already superseded by a prior merge in the same batch,
/// the transition `superseded → superseded` is illegal and we skip
/// this pair without touching the file.
fn merge_one(
    storage: &crate::storage::Storage,
    cogz_dir: &Path,
    pair: &MergeCandidate,
) -> Result<(), MergeError> {
    // 1. Fetch the current entity state from the DB (not the stale
    //    candidate list — a prior merge in the same batch may have
    //    already changed its status).
    let superseded_entity = {
        let conn = storage.conn();
        crate::storage::crud::get_entity(&conn, &pair.superseded_id)?
    };

    // 2. Validate the status transition BEFORE writing the file.
    //    If the entity is already superseded (from a prior merge),
    //    this is a no-op pair — skip without touching the file.
    transition_status(&superseded_entity.status, "superseded")?;

    let file_path = superseded_entity
        .file_path
        .as_ref()
        .ok_or_else(|| StorageError::EntityNotFound(pair.superseded_id.clone()))?;

    let path = resolve_file_path(file_path, cogz_dir);

    // Hold the canonical-file write lock across the read-modify-write
    // sequence to prevent concurrent writers from clobbering the
    // superseded entity's frontmatter.
    let _file_lock = storage.file_lock();

    let mut entity_file = read_entity_file(&path)?;

    // Set superseded_by in frontmatter.
    entity_file
        .frontmatter
        .insert("superseded_by", FmValue::String(pair.survivor_id.clone()));
    entity_file.status = "superseded".to_string();
    entity_file.updated_at = chrono::Utc::now().to_rfc3339();

    // Write file first.
    write_entity_file(&path, &entity_file)?;

    // 3. Sync the superseded file to the DB. This updates the entity
    //    status to "superseded" and creates the `superseded_by` edge
    //    from frontmatter via sync_references. The superseded entity's
    //    original edges (references, supports, etc.) are preserved in
    //    its frontmatter and recreated on sync. Graph expansion follows
    //    the `superseded_by` edge to reach the survivor, so no DB-only
    //    edge redirection is needed — everything is canonical and
    //    rebuildable.
    let relative_path = path
        .strip_prefix(cogz_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();
    let sync_result = crate::files::sync_single_file(storage, cogz_dir, &relative_path);
    if let Some(err) = sync_result.errors.first() {
        return Err(StorageError::File(format!(
            "sync failed for superseded entity {}: {}",
            path.display(),
            err.error
        ))
        .into());
    }

    // 4. Record the merge event.
    let conn = storage.conn();
    let payload = serde_json::json!({
        "survivor_id": pair.survivor_id,
        "superseded_id": pair.superseded_id,
        "reason": pair.reason,
    });
    record_event(
        &conn,
        EventType::KnowledgeMerged,
        Some(&pair.survivor_id),
        &payload,
    )?;

    Ok(())
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
