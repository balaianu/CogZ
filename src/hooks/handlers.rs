//! Lifecycle event handlers for file_save and session_end.
//!
//! `handle_file_save` handles two paths:
//! - Source files (not under `.cogz/`): incremental code reindex +
//!   stale-knowledge flagging.
//! - `.cogz/` entity files: incremental file sync + embedding, so
//!   human-edited knowledge/rules/observations reach the DB without
//!   a manual `cogz reindex`.
//!
//! `handle_session_end` runs consolidation (promotion + merge).

use std::path::Path;
use std::sync::Arc;

use crate::config::Config;
use crate::embed::{EmbeddingCache, EmbeddingModel, OnnxEmbeddingModel};
use crate::files::embed_sync::{embed_entities, store_embeddings};
use crate::files::sync_single_file;
use crate::hooks::lifecycle::{ConsolidationSummary, ReindexSummary};
use crate::storage::Storage;
use crate::storage::crud::get_entities_batch;

/// Handle a file_save event. The path determines which pipeline runs:
///
/// - **Source file** (not under `.cogz/`): incremental code reindex
///   via `reindex_code`, then `flag_stale_knowledge` for any changed
///   or deleted code entities.
///
/// - **`.cogz/` entity file**: incremental file sync via
///   `sync_incremental` (file → DB), then embed synced entities using
///   the knowledge model. This keeps the DB in sync when a human edits
///   knowledge/rules/observations directly, without requiring a manual
///   `cogz reindex`.
pub fn handle_file_save(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &Path,
    query_model: &OnnxEmbeddingModel,
    file_path: Option<&str>,
) -> ReindexSummary {
    let path = match file_path {
        Some(p) => p,
        None => {
            return ReindexSummary {
                reindexed: false,
                synced: false,
                created: 0,
                updated: 0,
                marked_stale: 0,
                stale_knowledge_flagged: 0,
                embedded: 0,
            };
        }
    };

    if path.starts_with(".cogz/") || path.contains("/.cogz/") || path.starts_with("./.cogz/") {
        return handle_cogz_file_save(storage, cogz_dir, query_model, path);
    }

    handle_source_file_save(storage, config, cogz_dir, path)
}

/// Sync a `.cogz/` entity file to the DB and embed it. Syncs only
/// the saved file (not the entire `.cogz/` directory), then embeds
/// the entity using the knowledge model. Follows the lock discipline:
/// fetch under lock, embed without lock, store under lock.
fn handle_cogz_file_save(
    storage: &Arc<Storage>,
    cogz_dir: &Path,
    query_model: &OnnxEmbeddingModel,
    file_path: &str,
) -> ReindexSummary {
    tracing::debug!("file_save hook: syncing {file_path}");

    let result = sync_single_file(storage, cogz_dir, file_path);

    if !result.errors.is_empty() {
        tracing::warn!(
            "file_save sync: {} error(s), first: {}",
            result.errors.len(),
            result.errors[0].error
        );
    }

    // Embed synced entities. .cogz/ files are always knowledge-type
    // entities (observations, rules, knowledge), so the knowledge
    // model (query_model) is the correct one.
    let embedded = if result.synced_entity_ids.is_empty() || !query_model.is_available() {
        0
    } else {
        embed_synced_entities(storage, query_model, &result.synced_entity_ids)
    };

    ReindexSummary {
        reindexed: false,
        synced: true,
        created: result.created,
        updated: result.updated,
        marked_stale: result.marked_stale,
        stale_knowledge_flagged: 0,
        embedded,
    }
}

/// Embed entities that were synced. Fetches entity data under the DB
/// lock, drops the lock for ONNX inference, then re-acquires to store
/// vectors. Returns the count of entities successfully embedded.
fn embed_synced_entities(
    storage: &Arc<Storage>,
    query_model: &OnnxEmbeddingModel,
    entity_ids: &[String],
) -> usize {
    let cache = EmbeddingCache::new();

    // Phase 1: fetch entity data under the lock, then drop it.
    let entities: Vec<_> = {
        let conn = storage.conn();
        get_entities_batch(&conn, entity_ids).unwrap_or_default()
    };

    if entities.is_empty() {
        return 0;
    }

    // Phase 2: embed without holding the DB lock.
    let embeddings = embed_entities(query_model, &cache, &entities);

    if embeddings.is_empty() {
        return 0;
    }

    // Phase 3: store vectors under the lock.
    let mut conn = storage.conn();
    store_embeddings(&mut conn, &embeddings)
}

/// Reindex source code after a source file is saved. Runs
/// `reindex_code` (incremental, git-diff based) then flags stale
/// knowledge for any changed or deleted code entities.
fn handle_source_file_save(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &Path,
    path: &str,
) -> ReindexSummary {
    let repo_root = cogz_dir.parent().unwrap_or_else(|| Path::new("."));

    tracing::debug!("file_save hook: triggering reindex for {}", path);

    let result = crate::index::reindex_code(storage, repo_root, config);

    let mut all_changed = result.changed_code_ids.clone();
    all_changed.extend(result.deleted_code_ids.iter().cloned());
    let stale_flagged = if all_changed.is_empty() {
        0
    } else {
        crate::index::stale_flagging::flag_stale_knowledge(storage, cogz_dir, &all_changed)
    };

    ReindexSummary {
        reindexed: true,
        synced: false,
        created: result.created,
        updated: result.updated,
        marked_stale: result.marked_stale,
        stale_knowledge_flagged: stale_flagged,
        embedded: 0,
    }
}

/// Handle a session_end event: run consolidation (promotion + merge)
/// for real. The configured thresholds are the safety mechanism — if
/// they're met, the system should act. This aligns with the first
/// principle that consolidation is continuous, not batch. Rules
/// created by promotion are git-tracked and reviewable; merged
/// entities are superseded (not deleted) and remain in the graph.
pub fn handle_session_end(
    storage: &Arc<Storage>,
    config: &Config,
    cogz_dir: &Path,
) -> ConsolidationSummary {
    let promoted = match crate::consolidate::promote::run_promotion(
        storage,
        cogz_dir,
        &config.consolidation,
        false,
    ) {
        Ok(p) => p.len(),
        Err(e) => {
            tracing::warn!("session_end consolidation (promote) failed: {}", e);
            0
        }
    };

    let merged =
        match crate::consolidate::merge::run_merge(storage, cogz_dir, &config.consolidation, false)
        {
            Ok(m) => m.len(),
            Err(e) => {
                tracing::warn!("session_end consolidation (merge) failed: {}", e);
                0
            }
        };

    ConsolidationSummary {
        promotions: promoted,
        merges: merged,
    }
}
