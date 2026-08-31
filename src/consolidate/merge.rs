//! Duplicate merging — edge redirection and superseded marking.
//!
//! Confirmed duplicate observations are merged: one entity survives,
//! the other is marked `superseded` with a `superseded_by` property.
//! All edges pointing to or from the superseded entity are redirected
//! to the survivor. Knowledge is flagged, not auto-merged (human-curated).
//! Rules are surfaced for review, not auto-merged.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::config::ConsolidationConfig;
use crate::files::frontmatter::FmValue;
use crate::files::{FrontmatterError, read_entity_file, write_entity_file};
use crate::storage::StorageError;
use crate::storage::edges::{Edge, delete_edge, get_edges_from, get_edges_to, insert_edge};
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
pub fn run_merge(
    storage: &crate::storage::Storage,
    cogz_dir: &Path,
    config: &ConsolidationConfig,
    dry_run: bool,
) -> Result<Vec<MergeResult>, StorageError> {
    let pairs = find_merge_candidates(storage, config)?;

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

/// Find duplicate observation pairs via embedding similarity. The
/// earlier-created entity survives; the later one is superseded.
fn find_merge_candidates(
    storage: &crate::storage::Storage,
    config: &ConsolidationConfig,
) -> Result<Vec<MergeCandidate>, StorageError> {
    use crate::storage::embeddings::knn_search;

    let observations = {
        let conn = storage.conn();
        get_entities_by_type(&conn, "observation", Some("active"), 500)?
    };

    if observations.len() < 2 {
        return Ok(Vec::new());
    }

    let mut seen_pairs = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    for obs in &observations {
        let conn = storage.conn();
        // Get this observation's embedding.
        let own_embedding = match get_embedding(&conn, &obs.id) {
            Some(e) => e,
            None => continue,
        };

        let neighbors = match knn_search(&conn, &own_embedding, 5) {
            Ok(n) => n,
            Err(_) => continue,
        };

        for (neighbor_id, distance) in &neighbors {
            if neighbor_id == &obs.id {
                continue;
            }
            let similarity = 1.0 / (1.0 + *distance as f64);
            if similarity < config.dedup_threshold {
                continue;
            }

            // Only merge observations.
            let neighbor = match observations.iter().find(|o| o.id == *neighbor_id) {
                Some(o) => o,
                None => continue,
            };

            // Survivor = earlier created. Superseded = later created.
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

            candidates.push(MergeCandidate {
                survivor_id: survivor.id.clone(),
                superseded_id: superseded.id.clone(),
                reason: format!("{:.2} embedding similarity", similarity),
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
fn merge_one(
    storage: &crate::storage::Storage,
    cogz_dir: &Path,
    pair: &MergeCandidate,
) -> Result<(), MergeError> {
    // 1. Read the superseded entity's file, update frontmatter.
    let superseded_entity = {
        let conn = storage.conn();
        crate::storage::crud::get_entity(&conn, &pair.superseded_id)?
    };

    let file_path = superseded_entity
        .file_path
        .as_ref()
        .ok_or_else(|| StorageError::EntityNotFound(pair.superseded_id.clone()))?;

    let path = resolve_file_path(file_path, cogz_dir);
    let mut entity_file = read_entity_file(&path)?;

    // Set superseded_by in frontmatter.
    entity_file
        .frontmatter
        .insert("superseded_by", FmValue::String(pair.survivor_id.clone()));
    entity_file.status = "superseded".to_string();
    entity_file.updated_at = chrono::Utc::now().to_rfc3339();

    // Write file first.
    write_entity_file(&path, &entity_file)?;

    // 2. DB: redirect edges, update status.
    let conn = storage.conn();

    // Validate the status transition before applying.
    transition_status(&superseded_entity.status, "superseded")?;

    redirect_edges(&conn, &pair.superseded_id, &pair.survivor_id)?;

    // Update the superseded entity's status and properties via the
    // storage layer (no direct SQL outside storage/).
    let mut updated = superseded_entity.clone();
    if let Some(obj) = updated.properties.as_object_mut() {
        obj.insert(
            "superseded_by".to_string(),
            serde_json::Value::String(pair.survivor_id.clone()),
        );
    }
    updated.status = "superseded".to_string();
    updated.updated_at = chrono::Utc::now().to_rfc3339();
    crate::storage::crud::update_entity(&conn, &updated)?;

    // Record event.
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

/// Get an entity's embedding from the vec0 table.
fn get_embedding(conn: &Connection, entity_id: &str) -> Option<Vec<f32>> {
    crate::storage::embeddings::get_embedding(conn, entity_id)
        .ok()
        .flatten()
}

/// Redirect all edges pointing to or from `old_id` to `new_id`.
/// Edges that would create duplicates (same source, target, type) are
/// skipped. Self-loops are removed.
fn redirect_edges(conn: &Connection, old_id: &str, new_id: &str) -> Result<(), StorageError> {
    // Redirect incoming edges: old_id as target → new_id as target.
    let incoming = get_edges_to(conn, old_id)?;
    for edge in &incoming {
        // Skip self-loops.
        if edge.source_id == new_id {
            delete_edge(conn, &edge.source_id, old_id, &edge.edge_type)?;
            continue;
        }
        // Insert the redirected edge (skip if it already exists).
        insert_edge(
            conn,
            &Edge {
                source_id: edge.source_id.clone(),
                target_id: new_id.to_string(),
                edge_type: edge.edge_type.clone(),
                weight: edge.weight,
                created_at: edge.created_at.clone(),
            },
        )
        .ok();
        delete_edge(conn, &edge.source_id, old_id, &edge.edge_type)?;
    }

    // Redirect outgoing edges: old_id as source → new_id as source.
    let outgoing = get_edges_from(conn, old_id)?;
    for edge in &outgoing {
        // Skip self-loops.
        if edge.target_id == new_id {
            delete_edge(conn, old_id, &edge.target_id, &edge.edge_type)?;
            continue;
        }
        insert_edge(
            conn,
            &Edge {
                source_id: new_id.to_string(),
                target_id: edge.target_id.clone(),
                edge_type: edge.edge_type.clone(),
                weight: edge.weight,
                created_at: edge.created_at.clone(),
            },
        )
        .ok();
        delete_edge(conn, old_id, &edge.target_id, &edge.edge_type)?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
