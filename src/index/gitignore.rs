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

/// Configuration for the source file scanner.
pub struct ScanConfig<'a> {
    /// Repo root directory.
    pub root: &'a Path,
    /// Explicit allow-list paths from `[index].allow` config.
    /// These are indexed despite being gitignored.
    pub allow: &'a [String],
}

/// Scan the repo for source files, respecting `.gitignore`.
///
/// Returns paths relative to `root`. Files in `.cogz/` and hidden
/// directories (other than `.gitignore` itself) are excluded —
/// `.cogz/` files are synced separately by the file sync layer.
///
/// The `allow` list overrides gitignore for matching paths. A path
/// in `allow` is indexed even if `.gitignore` would exclude it.
pub fn scan_source_files(config: &ScanConfig<'_>) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(config.root);
    builder
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true);

    // Build allow-list glob matchers for the second pass.
    let allow_globs: Vec<_> = config
        .allow
        .iter()
        .filter_map(|pattern| {
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map(|g| g.compile_matcher())
                .map_err(|e| {
                    tracing::warn!("invalid allow pattern '{}': {}", pattern, e);
                })
                .ok()
        })
        .collect();

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
                // Check if this path matches any allow glob
                if allow_globs.iter().any(|g| g.is_match(rel)) {
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
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn scan_finds_rust_files() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        write_file(dir.path(), "src/lib.rs", "pub fn foo() {}");
        write_file(dir.path(), "README.md", "# Test");

        let files = scan_source_files(&ScanConfig {
            root: dir.path(),
            allow: &[],
        });
        assert_eq!(files.len(), 2);
        assert!(files.contains(&PathBuf::from("src/main.rs")));
        assert!(files.contains(&PathBuf::from("src/lib.rs")));
    }

    #[test]
    fn scan_finds_python_files() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "app.py", "def main(): pass");
        write_file(dir.path(), "pkg/__init__.py", "");
        write_file(dir.path(), "data.json", "{}");

        let files = scan_source_files(&ScanConfig {
            root: dir.path(),
            allow: &[],
        });
        assert_eq!(files.len(), 2);
        assert!(files.contains(&PathBuf::from("app.py")));
        assert!(files.contains(&PathBuf::from("pkg/__init__.py")));
    }

    #[test]
    fn scan_respects_gitignore() {
        let dir = TempDir::new().unwrap();
        // The ignore crate requires a .git directory to apply gitignore rules
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        write_file(dir.path(), "target/release.rs", "fn release() {}");
        write_file(dir.path(), ".gitignore", "target/\n");

        let files = scan_source_files(&ScanConfig {
            root: dir.path(),
            allow: &[],
        });
        assert_eq!(files.len(), 1);
        assert!(files.contains(&PathBuf::from("src/main.rs")));
    }

    #[test]
    fn scan_excludes_cogz_dir() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        write_file(dir.path(), ".cogz/knowledge/test.rs", "fn test() {}");

        let files = scan_source_files(&ScanConfig {
            root: dir.path(),
            allow: &[],
        });
        assert_eq!(files.len(), 1);
        assert!(files.contains(&PathBuf::from("src/main.rs")));
    }

    #[test]
    fn scan_allow_overrides_gitignore() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        write_file(dir.path(), "generated/code.rs", "fn gen() {}");
        write_file(dir.path(), ".gitignore", "generated/\n");

        // Without allow — generated/ is excluded
        let files = scan_source_files(&ScanConfig {
            root: dir.path(),
            allow: &[],
        });
        assert_eq!(files.len(), 1);

        // With allow — generated/code.rs is included
        let files = scan_source_files(&ScanConfig {
            root: dir.path(),
            allow: &["generated/*.rs".to_string()],
        });
        assert_eq!(files.len(), 2);
        assert!(files.contains(&PathBuf::from("generated/code.rs")));
    }

    #[test]
    fn language_for_extension_works() {
        assert_eq!(language_for_path(Path::new("foo.rs")), Some("rust"));
        assert_eq!(language_for_path(Path::new("bar.py")), Some("python"));
        assert_eq!(language_for_path(Path::new("baz.js")), None);
    }
}
