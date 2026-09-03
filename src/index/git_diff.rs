//! Git diff-based change detection for incremental reindexing.
//!
//! Compares the baseline commit (stored in meta as
//! `last_indexed_commit`) against the working directory to find
//! source files that were added, modified, or deleted since the last
//! index. Falls back to `None` (signalling a full scan) when the repo
//! is not a git repo, has no commits, or has no stored baseline.

use std::path::{Path, PathBuf};

/// Type of change detected for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
}

/// A file that changed since the baseline commit.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    /// Path relative to the repo root.
    pub path: PathBuf,
    pub change: ChangeType,
}

/// Detect changed source files since the baseline commit.
///
/// Diffs the baseline tree against the working directory (not HEAD),
/// so uncommitted changes are included. Also includes untracked files
/// via `repo.statuses()`.
///
/// Returns `None` if git is unavailable, the repo has no commits, or
/// `baseline_sha` is `None` (first index). The caller should do a
/// full scan in that case.
///
/// Only files matching supported source extensions (`.rs`, `.py`,
/// `.go`, `.js`, `.mjs`, `.cjs`, `.jsx`, `.ts`, `.tsx`, `.sh`, `.bash`)
/// are returned — other changes are irrelevant to code indexing.
pub fn changed_source_files(
    repo_root: &Path,
    baseline_sha: Option<&str>,
) -> Option<Vec<ChangedFile>> {
    let repo = git2::Repository::discover(repo_root).ok()?;
    let baseline = baseline_sha?;

    let baseline_tree = repo.revparse_single(baseline).ok()?.peel_to_tree().ok()?;
    let head_tree = repo.head().ok()?.peel_to_tree().ok()?;

    let mut changed = Vec::new();

    // Diff baseline tree → working directory (includes uncommitted).
    let diff = repo
        .diff_tree_to_workdir_with_index(Some(&baseline_tree), None)
        .ok()?;

    // Also diff baseline → HEAD to catch committed-but-not-worked-on changes.
    let committed_diff = repo
        .diff_tree_to_tree(Some(&baseline_tree), Some(&head_tree), None)
        .ok()?;

    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    let collect = |d: &git2::Diff,
                   changed: &mut Vec<ChangedFile>,
                   seen: &mut std::collections::HashSet<PathBuf>| {
        for delta in d.deltas() {
            let path = delta.new_file().path().or_else(|| delta.old_file().path());
            if let Some(p) = path {
                if !is_source_file(p) {
                    continue;
                }
                if seen.insert(p.to_path_buf()) {
                    let change = match delta.status() {
                        git2::Delta::Added => ChangeType::Added,
                        git2::Delta::Deleted => ChangeType::Deleted,
                        _ => ChangeType::Modified,
                    };
                    changed.push(ChangedFile {
                        path: p.to_path_buf(),
                        change,
                    });
                }
            }
        }
    };

    collect(&diff, &mut changed, &mut seen);
    collect(&committed_diff, &mut changed, &mut seen);

    // Untracked files (new files not yet in the index).
    if let Ok(statuses) = repo.statuses(None) {
        for entry in statuses.iter() {
            if entry.status() == git2::Status::WT_NEW
                && let Some(p) = entry.path()
            {
                let path = PathBuf::from(p);
                if !is_source_file(&path) {
                    continue;
                }
                if seen.insert(path.clone()) {
                    changed.push(ChangedFile {
                        path,
                        change: ChangeType::Added,
                    });
                }
            }
        }
    }

    changed.sort_by(|a, b| a.path.cmp(&b.path));
    Some(changed)
}

/// Get the current HEAD commit SHA, or `None` if not a git repo or
/// no commits exist.
pub fn head_sha(repo_root: &Path) -> Option<String> {
    let repo = git2::Repository::discover(repo_root).ok()?;
    let head = repo.head().ok()?;
    let commit = head.peel_to_commit().ok()?;
    Some(commit.id().to_string())
}

fn is_source_file(path: &Path) -> bool {
    crate::index::gitignore::is_source_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_git_repo(dir: &Path) {
        git2::Repository::init(dir).unwrap();
        let repo = git2::Repository::open(dir).unwrap();
        // Set a minimal identity so commits work.
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();
    }

    fn commit_all(dir: &Path) -> String {
        let repo = git2::Repository::open(dir).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();

        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();

        let sig = repo.signature().unwrap();
        let head = repo.head().ok();
        let parents: Vec<_> = head
            .iter()
            .filter_map(|h| h.peel_to_commit().ok())
            .collect();

        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parent_refs)
            .unwrap();
        commit.to_string()
    }

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn no_baseline_returns_none() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        let sha = commit_all(dir.path());

        let result = changed_source_files(dir.path(), None);
        assert!(result.is_none());

        // head_sha should still work.
        assert_eq!(head_sha(dir.path()), Some(sha));
    }

    #[test]
    fn detects_modified_file() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        let sha = commit_all(dir.path());

        // Modify the file.
        write_file(dir.path(), "src/main.rs", "fn main() { println!(\"hi\"); }");

        let changed = changed_source_files(dir.path(), Some(&sha)).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(changed[0].change, ChangeType::Modified);
    }

    #[test]
    fn detects_added_file() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        let sha = commit_all(dir.path());

        // Add a new file (untracked).
        write_file(dir.path(), "src/lib.rs", "pub fn foo() {}");

        let changed = changed_source_files(dir.path(), Some(&sha)).unwrap();
        let added: Vec<_> = changed
            .iter()
            .filter(|c| c.change == ChangeType::Added)
            .collect();
        assert!(
            added
                .iter()
                .any(|c| c.path == std::path::Path::new("src/lib.rs"))
        );
    }

    #[test]
    fn detects_deleted_file() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        write_file(dir.path(), "src/lib.rs", "pub fn foo() {}");
        let sha = commit_all(dir.path());

        // Delete a file.
        fs::remove_file(dir.path().join("src/lib.rs")).unwrap();

        let changed = changed_source_files(dir.path(), Some(&sha)).unwrap();
        let deleted: Vec<_> = changed
            .iter()
            .filter(|c| c.change == ChangeType::Deleted)
            .collect();
        assert!(
            deleted
                .iter()
                .any(|c| c.path == std::path::Path::new("src/lib.rs"))
        );
    }

    #[test]
    fn ignores_non_source_files() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        write_file(dir.path(), "README.md", "# test");
        let sha = commit_all(dir.path());

        // Modify both.
        write_file(dir.path(), "src/main.rs", "fn main() { }");
        write_file(dir.path(), "README.md", "# changed");

        let changed = changed_source_files(dir.path(), Some(&sha)).unwrap();
        assert!(
            changed
                .iter()
                .all(|c| c.path.extension().is_some_and(|e| e == "rs" || e == "py"))
        );
    }

    #[test]
    fn no_changes_returns_empty() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        write_file(dir.path(), "src/main.rs", "fn main() {}");
        let sha = commit_all(dir.path());

        let changed = changed_source_files(dir.path(), Some(&sha)).unwrap();
        assert!(changed.is_empty());
    }

    #[test]
    fn non_git_repo_returns_none() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "src/main.rs", "fn main() {}");

        let result = changed_source_files(dir.path(), Some("fake-sha"));
        assert!(result.is_none());

        assert!(head_sha(dir.path()).is_none());
    }
}
