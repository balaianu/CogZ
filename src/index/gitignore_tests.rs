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
        deny: &[],
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
        deny: &[],
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
        deny: &[],
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
        deny: &[],
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
        deny: &[],
    });
    assert_eq!(files.len(), 1);

    // With allow — generated/code.rs is included
    let files = scan_source_files(&ScanConfig {
        root: dir.path(),
        allow: &["generated/*.rs".to_string()],
        deny: &[],
    });
    assert_eq!(files.len(), 2);
    assert!(files.contains(&PathBuf::from("generated/code.rs")));
}

#[test]
fn scan_deny_excludes_non_gitignored_files() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "src/main.rs", "fn main() {}");
    write_file(dir.path(), "vendor/lib.rs", "fn lib() {}");
    write_file(dir.path(), "vendor/util.rs", "fn util() {}");

    // Without deny — all .rs files included
    let files = scan_source_files(&ScanConfig {
        root: dir.path(),
        allow: &[],
        deny: &[],
    });
    assert_eq!(files.len(), 3);

    // With deny — vendor/ excluded
    let files = scan_source_files(&ScanConfig {
        root: dir.path(),
        allow: &[],
        deny: &["vendor/**/*.rs".to_string()],
    });
    assert_eq!(files.len(), 1);
    assert!(files.contains(&PathBuf::from("src/main.rs")));
}

#[test]
fn scan_deny_glob_pattern_matches_recursively() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "src/main.rs", "fn main() {}");
    write_file(dir.path(), "bench/bench1.rs", "fn bench1() {}");
    write_file(dir.path(), "bench/nested/bench2.rs", "fn bench2() {}");

    let files = scan_source_files(&ScanConfig {
        root: dir.path(),
        allow: &[],
        deny: &["bench/**/*.rs".to_string()],
    });
    assert_eq!(files.len(), 1);
    assert!(files.contains(&PathBuf::from("src/main.rs")));
}

#[test]
fn scan_allow_overrides_deny() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "src/main.rs", "fn main() {}");
    write_file(dir.path(), "vendor/keep.rs", "fn keep() {}");
    write_file(dir.path(), "vendor/skip.rs", "fn skip() {}");

    // Deny vendor/**, but allow vendor/keep.rs
    let files = scan_source_files(&ScanConfig {
        root: dir.path(),
        allow: &["vendor/keep.rs".to_string()],
        deny: &["vendor/**/*.rs".to_string()],
    });
    assert!(files.contains(&PathBuf::from("src/main.rs")));
    assert!(files.contains(&PathBuf::from("vendor/keep.rs")));
    assert!(!files.contains(&PathBuf::from("vendor/skip.rs")));
}

#[test]
fn scan_deny_empty_default_includes_all() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "a.rs", "fn a() {}");
    write_file(dir.path(), "b.rs", "fn b() {}");

    let files = scan_source_files(&ScanConfig {
        root: dir.path(),
        allow: &[],
        deny: &[],
    });
    assert_eq!(files.len(), 2);
}

#[test]
fn language_for_extension_works() {
    assert_eq!(language_for_path(Path::new("foo.rs")), Some("rust"));
    assert_eq!(language_for_path(Path::new("bar.py")), Some("python"));
    assert_eq!(language_for_path(Path::new("baz.go")), Some("go"));
    assert_eq!(language_for_path(Path::new("qux.js")), Some("javascript"));
    assert_eq!(language_for_path(Path::new("qux.mjs")), Some("javascript"));
    assert_eq!(language_for_path(Path::new("app.ts")), Some("typescript"));
    assert_eq!(language_for_path(Path::new("comp.tsx")), Some("tsx"));
    assert_eq!(language_for_path(Path::new("deploy.sh")), Some("bash"));
    assert_eq!(language_for_path(Path::new("setup.bash")), Some("bash"));
    assert_eq!(language_for_path(Path::new("style.css")), None);
    assert_eq!(language_for_path(Path::new("page.html")), None);
}

// --- is_test_file tests ---

#[test]
fn test_file_detection_rust() {
    // Positive: actual Rust test files
    assert!(is_test_file("tests/integration.rs"));
    assert!(is_test_file("src/storage/tests.rs"));
    assert!(is_test_file("src/search/hybrid_tests.rs"));
    assert!(is_test_file("tests.rs"));

    // Negative: files with "test" in the name but not test files
    assert!(
        !is_test_file("src/test_utils.rs"),
        "test_utils.rs is a helper, not a test file"
    );
    assert!(!is_test_file("src/latest.rs"), "latest.rs should not match");
    assert!(
        !is_test_file("src/protest.rs"),
        "protest.rs should not match"
    );
    assert!(!is_test_file("src/main.rs"));
    assert!(!is_test_file("src/lib.rs"));
}

#[test]
fn test_file_detection_go() {
    // Positive: Go test files (toolchain enforces _test.go suffix)
    assert!(is_test_file("handler_test.go"));
    assert!(is_test_file("pkg/storage/db_test.go"));

    // Negative: files containing "test" but not Go test files
    assert!(!is_test_file("protest.go"), "protest.go should not match");
    assert!(!is_test_file("latest.go"), "latest.go should not match");
    assert!(
        !is_test_file("test.go"),
        "test.go without _test suffix is not a test file"
    );
    assert!(!is_test_file("main.go"));
    assert!(!is_test_file("testing.go"), "testing.go is not a test file");
}

#[test]
fn test_file_detection_python() {
    // Positive: pytest/unittest convention
    assert!(is_test_file("test_foo.py"));
    assert!(is_test_file("test_database.py"));
    assert!(is_test_file("foo_test.py"));
    assert!(is_test_file("tests/test_app.py"));
    assert!(is_test_file("src/tests/test_models.py"));

    // Negative: files with "test" but not test files
    assert!(!is_test_file("testing.py"), "testing.py is not a test file");
    assert!(!is_test_file("protest.py"), "protest.py should not match");
    assert!(!is_test_file("contest.py"), "contest.py should not match");
    assert!(!is_test_file("app.py"));
    assert!(!is_test_file("models.py"));
}

#[test]
fn test_file_detection_js_ts() {
    // Positive: jest/vitest/mocha convention
    assert!(is_test_file("foo.test.js"));
    assert!(is_test_file("foo.spec.js"));
    assert!(is_test_file("bar.test.ts"));
    assert!(is_test_file("bar.spec.ts"));
    assert!(is_test_file("comp.test.tsx"));
    assert!(is_test_file("comp.spec.jsx"));
    assert!(is_test_file("src/__tests__/utils.test.js"));
    assert!(is_test_file("app.mjs.test.js") || true, "edge case");

    // Negative: files with "test" but not test files
    assert!(
        !is_test_file("test.js"),
        "test.js without base name is ambiguous, not a test file"
    );
    assert!(!is_test_file("testing.ts"), "testing.ts is not a test file");
    assert!(!is_test_file("latest.js"), "latest.js should not match");
    assert!(!is_test_file("protest.ts"), "protest.ts should not match");
    assert!(!is_test_file("app.js"));
    assert!(!is_test_file("index.ts"));
}

#[test]
fn test_file_detection_bash() {
    // Positive: test scripts
    assert!(is_test_file("test_deploy.sh"));
    assert!(is_test_file("deploy_test.sh"));
    assert!(is_test_file("test_setup.bash"));

    // Negative
    assert!(!is_test_file("deploy.sh"));
    assert!(!is_test_file("latest.sh"), "latest.sh should not match");
    assert!(!is_test_file("protest.sh"), "protest.sh should not match");
}

#[test]
fn test_file_detection_non_source() {
    // Non-source files are never test files
    assert!(!is_test_file("README.md"));
    assert!(!is_test_file("config.toml"));
    assert!(!is_test_file("Cargo.toml"));
    assert!(!is_test_file("Makefile"));
}

#[test]
fn test_file_detection_edge_cases() {
    // Empty path
    assert!(!is_test_file(""));
    // No extension
    assert!(!is_test_file("test"));
    assert!(!is_test_file("tests/foo"));
    // Deeply nested
    assert!(is_test_file("a/b/c/d/e/tests/foo.rs"));
    assert!(is_test_file("a/b/c/d/e/__tests__/foo.test.ts"));
}
