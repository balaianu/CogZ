//! Gitignore-aware source file scanner.
//!
//! Uses the `ignore` crate (same engine as ripgrep) to walk the repo
//! and yield source files that are not gitignored. The `[index].allow`
//! config list overrides gitignore for specific paths.

use std::path::{Path, PathBuf};

use globset::GlobBuilder;
use ignore::WalkBuilder;

/// Supported source file extensions and their language.
fn language_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "go" => Some("go"),
        "js" | "mjs" | "cjs" | "jsx" => Some("javascript"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "sh" | "bash" => Some("bash"),
        _ => None,
    }
}

/// Check if a path is a source file we can parse.
pub fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| language_for_extension(ext).is_some())
}

/// Get the language name for a source file path.
pub fn language_for_path(path: &Path) -> Option<&'static str> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(language_for_extension)
}

/// Check if a relative file path is a test file, using language-specific
/// conventions. This is the single source of truth for test-file
/// detection across search filtering and code-map generation.
///
/// Patterns are deliberately conservative — they match only well-known
/// test file naming conventions per language, not any file containing
/// the substring "test". This avoids excluding legitimate files like
/// `test_utils.rs` (a test helper, not a test file itself in Rust) or
/// `protest.go` (not a Go test file).
///
/// Conventions by language:
/// - **Rust**: `tests/` directory, `tests.rs`, `*_tests.rs` (integration
///   tests). Unit tests live inline in `#[cfg(test)] mod tests` and are
///   filtered by module title, not file path.
/// - **Go**: `*_test.go` — enforced by the Go toolchain.
/// - **Python**: `test_*.py`, `*_test.py` — pytest/unittest convention.
/// - **JS/TS/JSX/TSX**: `*.test.{ext}`, `*.spec.{ext}`, `__tests__/`
///   directory — jest/vitest/mocha convention.
/// - **Bash**: `test_*.sh`, `*_test.sh` — less standardized, but these
///   patterns cover the common cases without false positives.
pub fn is_test_file(rel_path: &str) -> bool {
    // Extract the file extension (lowercase, no leading dot).
    let ext = rel_path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext));

    let Some(ext) = ext else { return false };

    // Check directory-based patterns first (language-agnostic).
    // `tests/` at the top level or as a subdirectory.
    let in_tests_dir = rel_path.starts_with("tests/") || rel_path.contains("/tests/");

    match ext {
        // Rust: tests/ dir, tests.rs, *_tests.rs
        "rs" => {
            in_tests_dir
                || rel_path.ends_with("/tests.rs")
                || rel_path == "tests.rs"
                || rel_path.ends_with("_tests.rs")
        }
        // Go: *_test.go (Go toolchain enforces this)
        "go" => rel_path.ends_with("_test.go"),
        // Python: test_*.py, *_test.py, tests/ dir
        "py" => in_tests_dir || has_test_prefix(rel_path, "py") || has_test_suffix(rel_path, "py"),
        // JS/TS: *.test.{ext}, *.spec.{ext}, __tests__/ dir
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => {
            rel_path.contains("/__tests__/") || has_dot_test_or_spec(rel_path, ext)
        }
        // Bash: test_*.sh, *_test.sh
        "sh" | "bash" => has_test_prefix(rel_path, ext) || has_test_suffix(rel_path, ext),
        _ => false,
    }
}

/// Check if a file name starts with `test_` (e.g. `test_foo.py`).
/// Matches the filename only, not directory components.
fn has_test_prefix(rel_path: &str, ext: &str) -> bool {
    let filename = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let prefix = "test_";
    let suffix = format!(".{ext}");
    filename.starts_with(prefix)
        && filename.ends_with(&suffix)
        && filename.len() > prefix.len() + suffix.len()
}

/// Check if a file name ends with `_test.{ext}` (e.g. `foo_test.py`).
/// Matches the filename only, not directory components.
fn has_test_suffix(rel_path: &str, ext: &str) -> bool {
    let filename = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let suffix = format!("_test.{ext}");
    filename.ends_with(&suffix) && filename.len() > suffix.len()
}

/// Check if a file matches `*.test.{ext}` or `*.spec.{ext}` (JS/TS convention).
/// Ensures there's a base name before the `.test`/`.spec` part.
fn has_dot_test_or_spec(rel_path: &str, ext: &str) -> bool {
    let filename = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let test_suffix = format!(".test.{ext}");
    let spec_suffix = format!(".spec.{ext}");
    (filename.ends_with(&test_suffix) && filename.len() > test_suffix.len())
        || (filename.ends_with(&spec_suffix) && filename.len() > spec_suffix.len())
}

/// Configuration for the source file scanner.
pub struct ScanConfig<'a> {
    /// Repo root directory.
    pub root: &'a Path,
    /// Glob patterns from `[index].allow` config. Files matching
    /// these are indexed despite being gitignored or denied.
    pub allow: &'a [String],
    /// Glob patterns from `[index].deny` config. Files matching
    /// these are excluded from indexing even if not gitignored.
    /// Allow patterns take precedence over deny.
    pub deny: &'a [String],
}

/// Scan the repo for source files, respecting `.gitignore`.
///
/// Returns paths relative to `root`. Files in `.cogz/` and hidden
/// directories (other than `.gitignore` itself) are excluded —
/// `.cogz/` files are synced separately by the file sync layer.
///
/// Filter precedence: gitignore → deny → allow. A file matching both
/// `allow` and `deny` is indexed (allow wins). A file matching `deny`
/// but not `allow` is excluded even if not gitignored.
pub fn scan_source_files(config: &ScanConfig<'_>) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(config.root);
    builder
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true);

    // Build glob matchers for allow and deny lists.
    let build_matchers = |patterns: &[String], label: &str| {
        patterns
            .iter()
            .filter_map(|pattern| {
                GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map(|g| g.compile_matcher())
                    .map_err(|e| {
                        tracing::warn!("invalid {label} pattern '{}': {}", pattern, e);
                    })
                    .ok()
            })
            .collect::<Vec<_>>()
    };
    let allow_globs = build_matchers(config.allow, "allow");
    let deny_globs = build_matchers(config.deny, "deny");

    let is_denied = |rel: &Path| deny_globs.iter().any(|g| g.is_match(rel));
    let is_allowed = |rel: &Path| allow_globs.iter().any(|g| g.is_match(rel));

    let walker = builder.build();

    let mut files = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Skip .cogz/ directory — handled by file sync layer
        if path
            .strip_prefix(config.root)
            .is_ok_and(|rel| rel.starts_with(".cogz"))
        {
            continue;
        }

        if is_source_file(path)
            && let Ok(rel) = path.strip_prefix(config.root)
        {
            // Deny filter: exclude unless allow overrides.
            if is_denied(rel) && !is_allowed(rel) {
                continue;
            }
            files.push(rel.to_path_buf());
        }
    }

    // Second pass: add allow-listed files that were gitignored.
    // Walk without gitignore filtering to find files matching allow patterns.
    if !allow_globs.is_empty() {
        let mut allow_builder = WalkBuilder::new(config.root);
        allow_builder
            .hidden(true)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .parents(false);

        let allow_walker = allow_builder.build();
        for entry in allow_walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_file() || !is_source_file(path) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(config.root) {
                // Skip .cogz/ directory
                if rel.starts_with(".cogz") {
                    continue;
                }
                // Allow overrides both gitignore and deny.
                if is_allowed(rel) {
                    let rel_path = rel.to_path_buf();
                    if !files.contains(&rel_path) {
                        files.push(rel_path);
                    }
                }
            }
        }
    }

    files.sort();
    files
}

#[cfg(test)]
#[path = "gitignore_tests.rs"]
mod tests;
