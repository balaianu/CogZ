//! Auto-linking — automatically connect knowledge entities to code
//! entities by scanning content for file paths and symbol names.
//!
//! Two strategies:
//! 1. Path-based: scan content for file paths (e.g. `src/embed/onnx.rs`)
//!    and link to the corresponding file entity.
//! 2. Name-based: scan content for code entity titles (function/class
//!    names) and link to matching entities. Only unique names are
//!    linked to avoid false positives from common names.
//!
//! All auto-links are `references` edges from the knowledge entity to
//! the code entity. They are DB-only (not in frontmatter) and fully
//! rebuildable on reindex.

use std::collections::HashMap;

use crate::storage;
use crate::storage::edges::Edge;
use crate::storage::query::get_entities_by_type;

/// Sync auto-links: scan all knowledge entities (observation, rule,
/// knowledge) for references to code entities and create `references`
/// edges. Existing auto-links are cleared and rebuilt.
///
/// Must be called after both file sync and code sync are complete so
/// that all entities exist in the DB.
pub fn sync_auto_links(storage: &storage::Storage) -> usize {
    let conn = storage.conn();

    // Build name → code entity ID map. Only include names that are
    // unique across the codebase to avoid false positives.
    let code_types = ["function", "class", "file", "module"];
    let mut name_count: HashMap<String, usize> = HashMap::new();
    let mut name_to_id: HashMap<String, String> = HashMap::new();

    for entity_type in &code_types {
        if let Ok(entities) = get_entities_by_type(&conn, entity_type, None, 100_000) {
            for entity in entities {
                if let Some(ref title) = entity.title {
                    *name_count.entry(title.clone()).or_default() += 1;
                    name_to_id.entry(title.clone()).or_insert(entity.id.clone());
                }
            }
        }
    }

    // Filter to unique names only. Also build a set of file paths
    // for path-based matching.
    let unique_names: HashMap<String, String> = name_to_id
        .into_iter()
        .filter(|(name, _)| {
            let count = name_count.get(name).copied().unwrap_or(0);
            // Link by full title if unique. For file entities, the
            // title is the file name (e.g. "onnx.rs") which may not
            // be unique — require the full path instead.
            count == 1
        })
        .collect();

    // Build file path → file entity ID map for path-based matching.
    let mut path_to_id: HashMap<String, String> = HashMap::new();
    if let Ok(files) = get_entities_by_type(&conn, "file", None, 100_000) {
        for file in files {
            if let Some(ref fp) = file.file_path {
                path_to_id.insert(fp.clone(), file.id.clone());
            }
        }
    }

    // Scan knowledge entities for references.
    // Use Aho-Corasick to search all patterns in a single pass per
    // entity, reducing complexity from O(k × (p + n)) substring scans
    // to O(k × content_length + total_pattern_length).
    let knowledge_types = ["observation", "rule", "knowledge"];
    let mut edges: Vec<Edge> = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();

    // Build the pattern list and a mapping from pattern index to
    // code entity ID. Combine paths and names into one pattern set.
    let mut patterns: Vec<String> = Vec::new();
    let mut pattern_to_id: Vec<String> = Vec::new();
    // Track which patterns are name-based (need word boundary check)
    // vs path-based (already specific enough).
    let mut pattern_is_name: Vec<bool> = Vec::new();

    for (path, id) in &path_to_id {
        patterns.push(path.clone());
        pattern_to_id.push(id.clone());
        pattern_is_name.push(false);
    }
    for (name, id) in &unique_names {
        // Minimum length filters out short generic names. Modules get
        // a higher threshold (8) because module names like "storage",
        // "query", "model" are common English words. Functions and
        // classes get 4 — their names are usually more specific
        // (e.g. "knn_search", "build_sql").
        let min_len = 8; // Conservative: applies to all name-based matches
        if name.len() >= min_len && !is_stopword(name) {
            patterns.push(name.clone());
            pattern_to_id.push(id.clone());
            pattern_is_name.push(true);
        }
    }

    let ac = match aho_corasick::AhoCorasick::new(&patterns) {
        Ok(ac) => ac,
        Err(e) => {
            tracing::warn!("failed to build Aho-Corasick automaton: {}", e);
            return 0;
        }
    };

    for kt in &knowledge_types {
        if let Ok(entities) = get_entities_by_type(&conn, kt, Some("active"), 100_000) {
            for entity in entities {
                let content = format!(
                    "{}\n{}",
                    entity.title.as_deref().unwrap_or(""),
                    entity.content
                );

                // Single pass over content for all patterns.
                // For name-based patterns, verify word boundaries to
                // avoid matching substrings (e.g. "model" inside
                // "modeling"). Path-based patterns are specific enough
                // to skip this check.
                let mut matched_ids: Vec<String> = Vec::new();
                for mat in ac.find_iter(&content) {
                    let pat_idx = mat.pattern();
                    if pattern_is_name[pat_idx] {
                        let start = mat.start();
                        let end = mat.end();
                        if !is_word_boundary(&content, start, end) {
                            continue;
                        }
                    }
                    matched_ids.push(pattern_to_id[pat_idx].clone());
                }

                for target_id in matched_ids {
                    edges.push(Edge {
                        source_id: entity.id.clone(),
                        target_id,
                        edge_type: "auto_references".to_string(),
                        weight: 1.0,
                        created_at: now.clone(),
                    });
                }
            }
        }
    }

    // Deduplicate edges (a knowledge entity may match both path and name).
    edges.sort_by(|a, b| (&a.source_id, &a.target_id).cmp(&(&b.source_id, &b.target_id)));
    edges.dedup_by(|a, b| a.source_id == b.source_id && a.target_id == b.target_id);

    let count = edges.len();

    // The DELETE of old auto-links and INSERT of new ones are in a
    // single transaction so a COMMIT failure rolls back both —
    // otherwise the DELETE would persist (autocommit) while the
    // INSERTs roll back, silently destroying all auto-link edges.
    let in_transaction = conn.execute_batch("BEGIN").is_ok();
    if !in_transaction {
        tracing::warn!("failed to begin auto-link transaction — falling back to autocommit");
    }

    if let Err(e) = storage::edges::delete_edges_by_type(&conn, &["auto_references"]) {
        tracing::warn!("failed to clear auto-link edges: {}", e);
    }

    for edge in &edges {
        if let Err(e) = storage::edges::insert_edge_skip_fk_violation(&conn, edge) {
            tracing::debug!("skipped auto-link edge: {}", e);
        }
    }
    if in_transaction && let Err(e) = conn.execute_batch("COMMIT") {
        tracing::warn!("failed to commit auto-link transaction: {}", e);
        // Transaction rolled back — none of the edges persisted.
        return 0;
    }

    count
}

/// Common English words that are also common code entity names.
/// These are excluded from name-based auto-linking because they
/// produce false positives in almost every knowledge entry.
fn is_stopword(name: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        "access",
        "action",
        "add",
        "assemble",
        "batch",
        "block",
        "budget",
        "buffer",
        "build",
        "byte",
        "cache",
        "channel",
        "char",
        "check",
        "chunk",
        "clear",
        "client",
        "clone",
        "close",
        "code",
        "commands",
        "compress",
        "config",
        "content",
        "context",
        "copy",
        "count",
        "create",
        "crud",
        "cursor",
        "data",
        "date",
        "debug",
        "decay",
        "delete",
        "describe",
        "doctor",
        "download",
        "drop",
        "edge",
        "embeddings",
        "entities",
        "entry",
        "error",
        "event",
        "events",
        "exec",
        "expansion",
        "fetch",
        "field",
        "file",
        "file_path",
        "files",
        "filter",
        "find",
        "format",
        "frontmatter",
        "fusion",
        "get",
        "graph",
        "handle",
        "handlers",
        "hash",
        "hooks",
        "hop",
        "incremental",
        "index",
        "inference",
        "info",
        "init",
        "input",
        "insert",
        "issue",
        "item",
        "join",
        "json",
        "knowledge",
        "leaf",
        "limit",
        "line",
        "list",
        "load",
        "main",
        "match",
        "merge",
        "message",
        "migrations",
        "model",
        "name",
        "node",
        "offset",
        "open",
        "output",
        "pack",
        "page",
        "panic",
        "parse",
        "path",
        "print",
        "priority",
        "process",
        "query",
        "rank",
        "read",
        "record",
        "registry",
        "relevance",
        "remove",
        "report",
        "request",
        "reset",
        "response",
        "result",
        "root",
        "run",
        "save",
        "scanner",
        "schema",
        "score",
        "search",
        "section",
        "seed",
        "server",
        "service",
        "set",
        "settings",
        "size",
        "sort",
        "source",
        "split",
        "start",
        "state",
        "status",
        "stop",
        "storage",
        "store",
        "stream",
        "summary",
        "sync",
        "table",
        "target",
        "test",
        "text",
        "time",
        "token",
        "tools",
        "total",
        "trace",
        "tree",
        "type",
        "unavailable",
        "update",
        "uuid",
        "value",
        "warn",
        "word",
        "write",
    ];
    let lower = name.to_ascii_lowercase();
    STOPWORDS.contains(&lower.as_str())
}

/// Check if a match at [start, end) in `content` is bounded by word
/// boundaries on both sides. A word boundary is the start/end of the
/// string or a non-alphanumeric character (including `_`).
fn is_word_boundary(content: &str, start: usize, end: usize) -> bool {
    let bytes = content.as_bytes();
    let before_ok = start == 0 || !is_word_char(bytes[start - 1]);
    let after_ok = end == bytes.len() || !is_word_char(bytes[end]);
    before_ok && after_ok
}

/// Check if a byte is a word character (alphanumeric or underscore).
/// Rust identifiers use [a-zA-Z0-9_], so we treat all of these as
/// word characters for boundary checking.
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopword_filters_generic_names() {
        assert!(is_stopword("storage"));
        assert!(is_stopword("query"));
        assert!(is_stopword("model"));
        assert!(is_stopword("STORAGE"));
        assert!(is_stopword("entities"));
        assert!(is_stopword("knowledge"));
        assert!(is_stopword("embeddings"));
        assert!(is_stopword("frontmatter"));
        assert!(!is_stopword("knn_search"));
        assert!(!is_stopword("build_sql"));
        assert!(!is_stopword("assemble_context"));
        assert!(!is_stopword("tree_sitter"));
        assert!(!is_stopword("code_graph"));
    }

    #[test]
    fn word_boundary_rejects_substring_matches() {
        // "model" inside "modeling" — no word boundary at end
        assert!(!is_word_boundary("we are modeling this", 7, 12));
        // "model" as a standalone word
        assert!(is_word_boundary("the model is good", 4, 9));
        // "model" at start of string
        assert!(is_word_boundary("model is good", 0, 5));
        // "model" at end of string
        assert!(is_word_boundary("the model", 4, 9));
        // "search" inside "researching" — no boundary at start
        assert!(!is_word_boundary("researching this", 2, 8));
    }

    #[test]
    fn word_boundary_handles_underscores() {
        // "build_sql" inside "build_sql_query" — underscore is a word char
        assert!(!is_word_boundary("build_sql_query", 0, 9));
        // "build_sql" as standalone
        assert!(is_word_boundary("use build_sql here", 4, 13));
    }
}
