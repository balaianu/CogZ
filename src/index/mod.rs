//! Code indexing — tree-sitter parsing, gitignore-aware scanning,
//! and code entity synchronization.
//!
//! Phase 8: extract functions, classes, files, and modules from
//! source code into the entity graph with structural edges.

pub mod code_graph;
pub mod gitignore;
pub mod sync;
pub mod tree_sitter;

use std::path::Path;

use crate::config::Config;
use crate::storage;
use crate::storage::crud::EntityType;

pub use sync::CodeSyncResult;

/// Index code entities from the repository.
///
/// Scans source files (respecting `.gitignore` + `[index].allow`),
/// parses them with tree-sitter, syncs code entities to the DB,
/// and extracts structural edges (calls, imports, extends).
///
/// Returns the sync result with counts for the CLI output.
pub fn index_code(
    storage: &storage::Storage,
    repo_root: &Path,
    config: &Config,
) -> CodeSyncResult {
    // Phase 1: scan for source files (no lock held).
    let source_paths = gitignore::scan_source_files(&gitignore::ScanConfig {
        root: repo_root,
        allow: &config.index.allow,
    });

    // Phase 2: read and parse all source files (no lock held).
    let mut source_files: Vec<(std::path::PathBuf, String, self::tree_sitter::Language)> =
        Vec::new();
    for rel_path in &source_paths {
        let abs_path = repo_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to read {}: {}", abs_path.display(), e);
                continue;
            }
        };
        let language = match gitignore::language_for_path(rel_path) {
            Some(lang) => match self::tree_sitter::Language::from_str(lang) {
                Some(l) => l,
                None => continue,
            },
            None => continue,
        };
        source_files.push((rel_path.clone(), source, language));
    }

    // Phase 3: sync code entities to DB (lock held).
    let result = sync::sync_code_entities(storage, repo_root, &source_files);

    // Phase 4: sync structural edges (lock held).
    code_graph::sync_code_edges(storage, repo_root, &source_files);

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
