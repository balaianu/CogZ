//! Code indexing — tree-sitter parsing, gitignore-aware scanning,
//! and code entity synchronization.
//!
//! Phase 8: extract functions, classes, files, and modules from
//! source code into the entity graph with structural edges.

pub mod auto_link;
pub mod code_graph;
pub mod git_diff;
pub mod gitignore;
pub mod stale_flagging;
pub mod sync;
pub mod tree_sitter;

use std::path::Path;

use crate::config::Config;
use crate::storage;
use crate::storage::crud::EntityType;

/// Normalize a path to use forward slashes regardless of platform.
/// Git stores paths with forward slashes internally; we do the same
/// so that code entity UUIDs are consistent across Linux, macOS, and
/// Windows. Without this, `src\main.rs` on Windows would produce a
/// different UUID than `src/main.rs` on Linux, breaking cross-platform
/// team collaboration.
pub fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Convert a Path to a normalized string with forward slashes.
pub fn path_to_string(path: &Path) -> String {
    normalize_path(&path.to_string_lossy())
}

pub use gitignore::is_test_file;
pub use sync::{CodeSyncResult, mark_stale_for_deleted_files};

/// Index code entities from the repository.
///
/// Scans source files (respecting `.gitignore` + `[index].allow`),
/// parses them with tree-sitter, syncs code entities to the DB,
/// and extracts structural edges (calls, imports, extends).
///
/// Returns the sync result with counts for the CLI output.
pub fn index_code(storage: &storage::Storage, repo_root: &Path, config: &Config) -> CodeSyncResult {
    // Phase 1: scan for source files (no lock held).
    let source_paths = gitignore::scan_source_files(&gitignore::ScanConfig {
        root: repo_root,
        allow: &config.index.allow,
        deny: &config.index.deny,
    });

    // Phase 2: read and parse all source files in a single pass.
    // extract_all produces both entities and raw edges from one AST
    // walk, eliminating the need for a second parse during edge sync.
    let mut entities_by_file: Vec<(String, Vec<self::tree_sitter::CodeEntity>)> = Vec::new();
    let mut raw_edges_by_file: Vec<(String, Vec<self::tree_sitter::RawEdge>)> = Vec::new();
    // Track files that failed to read or parse. These must be excluded
    // from stale marking — a read failure does not mean the file was
    // deleted, and marking its entities stale would lose valid edges.
    let mut failed_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    for rel_path in &source_paths {
        let abs_path = repo_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to read {}: {}", abs_path.display(), e);
                failed_paths.insert(path_to_string(rel_path));
                continue;
            }
        };
        let language = match gitignore::language_for_path(rel_path) {
            Some(lang) => match self::tree_sitter::Language::parse_str(lang) {
                Some(l) => l,
                None => {
                    failed_paths.insert(path_to_string(rel_path));
                    continue;
                }
            },
            None => {
                failed_paths.insert(path_to_string(rel_path));
                continue;
            }
        };

        let (entities, raw_edges) = self::tree_sitter::extract_all(rel_path, &source, language);
        let path_str = path_to_string(rel_path);
        entities_by_file.push((path_str.clone(), entities));
        raw_edges_by_file.push((path_str, raw_edges));
    }

    // Phase 3: sync code entities to DB (lock held). Pass failed_paths
    // so that stale marking excludes files that failed to read — their
    // entities should not be marked stale just because of an I/O error.
    let mut result = sync::sync_code_entities(storage, &entities_by_file, &failed_paths);
    result.failed_files = failed_paths.len();

    // Phase 4: sync structural edges (lock held).
    // Skip the edge rebuild when any source file failed to read —
    // sync_code_edges deletes all existing edges before rebuilding,
    // which would remove outgoing edges from preserved entities.
    // Aborting the full rebuild keeps the previous edge set intact
    // until the next successful full index.
    if failed_paths.is_empty() {
        code_graph::sync_code_edges(storage, &entities_by_file, &raw_edges_by_file);
    } else {
        tracing::warn!(
            "skipping structural edge rebuild due to {} failed source read(s)",
            failed_paths.len()
        );
    }

    // Phase 5: auto-link knowledge entities to code entities.
    let link_count = auto_link::sync_auto_links(storage);
    if link_count > 0 {
        tracing::info!("auto-linked {} knowledge→code edges", link_count);
    }

    result
}

/// Count code entities by type for status reporting.
pub fn count_code_entities(conn: &rusqlite::Connection) -> (usize, usize, usize, usize) {
    let count = |entity_type: &str| -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE type = ?1 AND status = 'active'",
            rusqlite::params![entity_type],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    };

    (
        count(EntityType::Function.as_str()),
        count(EntityType::Class.as_str()),
        count(EntityType::File.as_str()),
        count(EntityType::Module.as_str()),
    )
}

/// Result of an incremental code reindex.
#[derive(Debug, Default)]
pub struct ReindexResult {
    /// Whether the reindex was incremental (git diff) or fell back to full.
    pub incremental: bool,
    pub created: usize,
    pub updated: usize,
    pub marked_stale: usize,
    pub skipped: usize,
    /// IDs of code entities that were created or updated — used by
    /// the stale-knowledge flagging step to find affected observations/rules.
    pub changed_code_ids: Vec<String>,
    /// IDs of code entities that were marked stale (deleted files).
    pub deleted_code_ids: Vec<String>,
    pub synced_entity_ids: Vec<String>,
}

/// Incremental code reindex — only re-parses files that changed since
/// the last index (detected via git diff). Falls back to a full
/// `index_code` if git is unavailable or no baseline commit is stored.
///
/// After syncing changed entities, records the current HEAD SHA as
/// the new baseline for the next reindex.
pub fn reindex_code(
    storage: &storage::Storage,
    repo_root: &Path,
    config: &Config,
) -> ReindexResult {
    let conn = storage.conn();
    let baseline = storage::get_meta(&conn, "last_indexed_commit");
    drop(conn);

    let changed = git_diff::changed_source_files(repo_root, baseline.as_deref());

    match changed {
        Some(files) if !files.is_empty() => reindex_incremental(storage, repo_root, config, &files),
        Some(_files) => {
            // No changes — just update the baseline commit.
            if let Some(sha) = git_diff::head_sha(repo_root) {
                let conn = storage.conn();
                if let Err(e) = storage::set_meta(&conn, "last_indexed_commit", &sha) {
                    tracing::warn!("failed to record last_indexed_commit: {}", e);
                }
            }
            ReindexResult {
                incremental: true,
                ..Default::default()
            }
        }
        None => {
            // No git or no baseline — full scan.
            let result = index_code(storage, repo_root, config);
            // The full scan marks deleted code entities as stale via
            // mark_stale_code_entities, but CodeSyncResult doesn't
            // return their IDs. Query them so flag_stale_knowledge can
            // be notified — otherwise knowledge referencing deleted
            // code silently remains active in the no-git path.
            let deleted_code_ids = if result.marked_stale > 0 {
                query_stale_code_ids(storage)
            } else {
                Vec::new()
            };
            ReindexResult {
                incremental: false,
                created: result.created,
                updated: result.updated,
                marked_stale: result.marked_stale,
                skipped: result.skipped,
                synced_entity_ids: result.synced_entity_ids.clone(),
                changed_code_ids: result.synced_entity_ids,
                deleted_code_ids,
            }
        }
    }
}

fn reindex_incremental(
    storage: &storage::Storage,
    repo_root: &Path,
    _config: &Config,
    changed: &[git_diff::ChangedFile],
) -> ReindexResult {
    let mut result = ReindexResult {
        incremental: true,
        ..Default::default()
    };

    // Separate changed files into added/modified (need parsing) and deleted.
    let (to_parse, deleted): (Vec<_>, Vec<_>) = changed
        .iter()
        .partition(|f| f.change != git_diff::ChangeType::Deleted);

    // Read and parse only changed files in a single pass (no lock held).
    let mut entities_by_file: Vec<(String, Vec<self::tree_sitter::CodeEntity>)> = Vec::new();
    let mut source_files_for_edges: Vec<(std::path::PathBuf, String, self::tree_sitter::Language)> =
        Vec::new();
    let mut had_failures = false;
    for cf in &to_parse {
        let abs_path = repo_root.join(&cf.path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to read {}: {}", abs_path.display(), e);
                had_failures = true;
                continue;
            }
        };
        let language = match gitignore::language_for_path(&cf.path) {
            Some(lang) => match self::tree_sitter::Language::parse_str(lang) {
                Some(l) => l,
                None => continue,
            },
            None => continue,
        };
        let path_str = path_to_string(&cf.path);
        let (entities, _) = self::tree_sitter::extract_all(&cf.path, &source, language);
        entities_by_file.push((path_str, entities));
        source_files_for_edges.push((cf.path.clone(), source, language));
    }

    // Sync changed entities to DB (no full stale sweep).
    let sync_result = sync::sync_code_entities_incremental(storage, &entities_by_file);
    result.created = sync_result.created;
    result.updated = sync_result.updated;
    result.skipped = sync_result.skipped;
    result.synced_entity_ids = sync_result.synced_entity_ids.clone();
    result.changed_code_ids = sync_result.synced_entity_ids.clone();
    // Entities removed from changed files (renamed or deleted within
    // a file that still exists) are marked stale by the sync function.
    result.marked_stale = sync_result.marked_stale;
    result.deleted_code_ids = sync_result.removed_entity_ids;

    // Re-extract structural edges for changed files only (incremental).
    if !source_files_for_edges.is_empty() {
        code_graph::sync_code_edges_incremental(storage, &source_files_for_edges);
    }

    // Rebuild auto-links. Code entity names/paths may have changed,
    // so knowledge→code auto-references need to be resynced. This
    // is a full rebuild (clears and recreates all auto-link edges)
    // to ensure no stale links remain after incremental changes.
    auto_link::sync_auto_links(storage);

    // Mark deleted files' code entities as stale.
    let deleted_paths: Vec<std::path::PathBuf> = deleted.iter().map(|f| f.path.clone()).collect();
    if !deleted_paths.is_empty() {
        result.marked_stale += sync::mark_stale_for_deleted_files(storage, &deleted_paths);
        // Collect the IDs of stale-marked entities for knowledge flagging.
        // Chunk the query to stay below SQLite's 999-variable limit:
        // 4 type params + N path params per chunk, max 990 paths.
        let conn = storage.conn();
        let code_types = ["function", "class", "file", "module"];
        let type_placeholders = (0..code_types.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let deleted_path_strs: Vec<String> =
            deleted_paths.iter().map(|p| path_to_string(p)).collect();
        const PATH_CHUNK: usize = 990;
        for chunk in deleted_path_strs.chunks(PATH_CHUNK) {
            let path_placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id FROM entities \
                 WHERE type IN ({type_placeholders}) AND status = 'stale' \
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
                    result.deleted_code_ids.push(row);
                }
            }
        }
    }

    // Record the new baseline commit only if all changed files were
    // successfully processed. On partial failure, preserve the previous
    // baseline so the next incremental reindex retries the failed files.
    if !had_failures && let Some(sha) = git_diff::head_sha(repo_root) {
        let conn = storage.conn();
        if let Err(e) = storage::set_meta(&conn, "last_indexed_commit", &sha) {
            tracing::warn!("failed to record last_indexed_commit: {}", e);
        }
    }

    result
}

/// Query all stale code entity IDs (function, class, file, module).
/// Used by the full-scan reindex fallback to populate
/// `deleted_code_ids` so `flag_stale_knowledge` can be notified about
/// code entities that were marked stale because their source files no
/// longer exist on disk.
fn query_stale_code_ids(storage: &storage::Storage) -> Vec<String> {
    let conn = storage.conn();
    let code_types = ["function", "class", "file", "module"];
    let placeholders = (0..code_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id FROM entities \
         WHERE type IN ({placeholders}) AND status = 'stale'"
    );
    let params: Vec<&dyn rusqlite::ToSql> = code_types
        .iter()
        .map(|t| t as &dyn rusqlite::ToSql)
        .collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("failed to query stale code IDs: {}", e);
            return Vec::new();
        }
    };
    let rows = match stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to query stale code IDs: {}", e);
            return Vec::new();
        }
    };
    rows.flatten().collect()
}
