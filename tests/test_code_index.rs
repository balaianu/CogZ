//! Integration tests for Phase 8 code indexing.
//!
//! Tests the full code indexing flow: scan source files, parse with
//! tree-sitter, sync code entities to DB, extract structural edges,
//! verify search finds code entities, and verify rebuildability.

use std::path::Path;

use cogz::config::Config;
use cogz::index::{self, gitignore::ScanConfig, tree_sitter::Language};
use cogz::storage::{self, Storage, crud::EntityType};

fn default_config() -> Config {
    Config::default_for("test-project")
}

fn sync_code(storage: &Storage, files: &[(std::path::PathBuf, String, Language)]) {
    let result = cogz::index::sync::sync_code_entities(storage, Path::new("."), files);
    cogz::index::code_graph::sync_code_edges(storage, Path::new("."), files);
    // Record counts for debugging
    tracing::debug!(
        "sync: {} created, {} updated, {} stale, {} skipped",
        result.created,
        result.updated,
        result.marked_stale,
        result.skipped
    );
}

#[test]
fn rust_code_entities_in_db() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}

struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}
"#;
    let files = vec![(
        std::path::PathBuf::from("src/geometry.rs"),
        code.to_string(),
        Language::Rust,
    )];

    sync_code(&storage, &files);

    let conn = storage.conn();
    let functions =
        storage::query::get_entities_by_type(&conn, "function", Some("active"), 100).unwrap();
    let classes =
        storage::query::get_entities_by_type(&conn, "class", Some("active"), 100).unwrap();
    let files = storage::query::get_entities_by_type(&conn, "file", Some("active"), 100).unwrap();
    let modules =
        storage::query::get_entities_by_type(&conn, "module", Some("active"), 100).unwrap();

    // Should have: add, new (functions); Point struct + Point impl (classes);
    // file entity; no module (no mod declaration)
    assert!(
        functions.len() >= 2,
        "expected >= 2 functions, got {}",
        functions.len()
    );
    assert!(
        classes.len() >= 2,
        "expected >= 2 classes, got {}",
        classes.len()
    );
    assert_eq!(files.len(), 1, "expected 1 file entity");
    assert!(modules.is_empty(), "expected no modules");
}

#[test]
fn python_code_entities_in_db() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
def add(a, b):
    return a + b

class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def distance(self):
        return (self.x ** 2 + self.y ** 2) ** 0.5
"#;
    let files = vec![(
        std::path::PathBuf::from("src/geometry.py"),
        code.to_string(),
        Language::Python,
    )];

    sync_code(&storage, &files);

    let conn = storage.conn();
    let functions =
        storage::query::get_entities_by_type(&conn, "function", Some("active"), 100).unwrap();
    let classes =
        storage::query::get_entities_by_type(&conn, "class", Some("active"), 100).unwrap();
    let files = storage::query::get_entities_by_type(&conn, "file", Some("active"), 100).unwrap();

    // Should have: add, __init__, distance (functions); Point (class); file
    assert!(
        functions.len() >= 3,
        "expected >= 3 functions, got {}",
        functions.len()
    );
    assert_eq!(classes.len(), 1, "expected 1 class, got {}", classes.len());
    assert_eq!(files.len(), 1, "expected 1 file entity");
}

#[test]
fn python_extends_edge_correct() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
class Animal:
    pass

class Dog(Animal):
    pass

class Cat(Animal):
    pass
"#;
    let files = vec![(
        std::path::PathBuf::from("animals.py"),
        code.to_string(),
        Language::Python,
    )];

    sync_code(&storage, &files);

    let conn = storage.conn();
    let extends_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'extends'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(extends_count, 2, "Dog→Animal and Cat→Animal");
}

#[test]
fn python_calls_edge_correct() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
def helper():
    return 42

def caller():
    return helper()
"#;
    let files = vec![(
        std::path::PathBuf::from("calls.py"),
        code.to_string(),
        Language::Python,
    )];

    sync_code(&storage, &files);

    let conn = storage.conn();
    let calls_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'calls'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(calls_count >= 1, "expected caller→helper calls edge");
}

#[test]
fn rust_extends_edge_for_impl_trait() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
trait Show {
    fn show(&self);
}

struct Item;

impl Show for Item {
    fn show(&self) {}
}
"#;
    let files = vec![(
        std::path::PathBuf::from("item.rs"),
        code.to_string(),
        Language::Rust,
    )];

    sync_code(&storage, &files);

    let conn = storage.conn();
    let extends_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'extends'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(extends_count, 1, "Item→Show extends edge");
}

#[test]
fn gitignored_files_not_indexed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // The `ignore` crate requires a git repository to apply .gitignore
    // rules. Initialize a minimal git repo.
    std::fs::create_dir_all(root.join(".git")).unwrap();

    // Create a .gitignore that excludes src/secret.rs
    std::fs::write(root.join(".gitignore"), "src/secret.rs\n").unwrap();

    // Create source files
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("src/secret.rs"), "fn secret() {}\n").unwrap();

    let config = default_config();
    let source_paths = cogz::index::gitignore::scan_source_files(&ScanConfig {
        root,
        allow: &config.index.allow,
    });

    assert!(
        source_paths
            .iter()
            .any(|p| p == &std::path::PathBuf::from("src/main.rs"))
    );
    assert!(
        !source_paths
            .iter()
            .any(|p| p == &std::path::PathBuf::from("src/secret.rs")),
        "gitignored file should not be scanned"
    );
}

#[test]
fn allow_overrides_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".gitignore"), "src/secret.rs\n").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/secret.rs"), "fn secret() {}\n").unwrap();

    let config = Config {
        index: cogz::config::IndexConfig {
            allow: vec!["src/secret.rs".to_string()],
        },
        ..default_config()
    };

    let source_paths = cogz::index::gitignore::scan_source_files(&ScanConfig {
        root,
        allow: &config.index.allow,
    });

    assert!(
        source_paths
            .iter()
            .any(|p| p == &std::path::PathBuf::from("src/secret.rs")),
        "allow-list should override gitignore"
    );
}

#[test]
fn cogz_dir_not_indexed_as_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::create_dir_all(root.join(".cogz")).unwrap();
    std::fs::write(root.join(".cogz/config.toml"), "# config\n").unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

    let source_paths = cogz::index::gitignore::scan_source_files(&ScanConfig { root, allow: &[] });

    assert!(
        !source_paths.iter().any(|p| p.starts_with(".cogz")),
        ".cogz/ should not be indexed as source"
    );
    assert!(
        source_paths
            .iter()
            .any(|p| p == &std::path::PathBuf::from("main.rs"))
    );
}

#[test]
fn search_finds_code_entities() {
    use cogz::embed::{EmbeddingModel, MockEmbeddingModel};
    use cogz::search::{QueryEmbeddings, SearchParams, search};

    let storage = Storage::open_memory().unwrap();
    let code = r#"
fn build_sql_query(table: &str) -> String {
    format!("SELECT * FROM {}", table)
}
"#;
    let files = vec![(
        std::path::PathBuf::from("src/query.rs"),
        code.to_string(),
        Language::Rust,
    )];

    sync_code(&storage, &files);

    // Embed with mock model so vector search works
    let model = MockEmbeddingModel::new();
    let conn = storage.conn();
    let functions =
        storage::query::get_entities_by_type(&conn, "function", Some("active"), 100).unwrap();
    for func in &functions {
        let text = format!(
            "{}\n\n{}",
            func.title.as_deref().unwrap_or(""),
            func.content
        );
        let embeddings = model.embed(&[&text]).unwrap();
        storage::embeddings::insert_embedding(&conn, &func.id, "function", &embeddings[0]).unwrap();
    }

    // Search for "sql query" (FTS-only, no query embedding)
    let params = SearchParams {
        entity_type: None,
        status: None,
        limit: 10,
        expand: false,
        max_hops: 0,
    };
    let config = cogz::config::SearchConfig {
        fts_weight: 0.3,
        vec_weight: 0.4,
        code_vec_weight: 0.3,
        rrf_k: 60,
        max_results: 20,
    };
    let results = search(
        &conn,
        "sql query",
        QueryEmbeddings::none(),
        &params,
        &config,
    )
    .unwrap();

    // Should find the build_sql_query function via FTS
    assert!(
        !results.results.is_empty(),
        "search should find code entities"
    );
    let has_function = results
        .results
        .iter()
        .any(|r| r.entity.r#type == "function");
    assert!(
        has_function,
        "search results should include a function entity"
    );
}

#[test]
fn rebuildability_preserves_uuids() {
    let code = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // First index
    let storage1 = Storage::open_memory().unwrap();
    sync_code(&storage1, &files);
    let conn1 = storage1.conn();
    let ids_before: Vec<String> = conn1
        .prepare("SELECT id FROM entities ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    drop(conn1);

    // Second index (simulating reset + reindex)
    let storage2 = Storage::open_memory().unwrap();
    sync_code(&storage2, &files);
    let conn2 = storage2.conn();
    let ids_after: Vec<String> = conn2
        .prepare("SELECT id FROM entities ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert_eq!(
        ids_before, ids_after,
        "UUIDs must be deterministic across rebuilds"
    );
}

#[test]
fn stale_marking_when_source_removed() {
    let storage = Storage::open_memory().unwrap();
    let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let files = vec![(
        std::path::PathBuf::from("src/math.rs"),
        code.to_string(),
        Language::Rust,
    )];

    // Index
    sync_code(&storage, &files);

    // Verify active
    let conn = storage.conn();
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(active > 0);
    drop(conn);

    // Reindex with empty file list
    sync_code(&storage, &[]);

    // Verify all stale
    let conn = storage.conn();
    let stale: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE status = 'stale'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stale > 0, "entities should be marked stale");
    assert_eq!(active, 0, "no active entities after source removed");
}

#[test]
fn full_index_code_pipeline() {
    let storage = Storage::open_memory().unwrap();
    let config = default_config();

    // Use the full index_code pipeline (scan + parse + sync + edges)
    // We need a temp directory with source files
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Initialize a minimal git repo for gitignore support
    std::fs::create_dir_all(root.join(".git")).unwrap();

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        r#"
fn main() {
    let result = compute(42);
    println!("{}", result);
}

fn compute(x: i32) -> i32 {
    x * 2
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.py"),
        r#"
def greet(name):
    return f"Hello, {name}"

class Greeter:
    def __init__(self, prefix):
        self.prefix = prefix

    def greet(self, name):
        return greet(f"{self.prefix} {name}")
"#,
    )
    .unwrap();

    let result = index::index_code(&storage, root, &config);

    assert!(result.created > 0, "should have created code entities");

    let conn = storage.conn();
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap();
    assert!(total > 0, "DB should have code entities");

    // Verify we have both Rust and Python entities
    let rust_files: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE file_path LIKE '%.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let py_files: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE file_path LIKE '%.py'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(rust_files > 0, "should have Rust entities");
    assert!(py_files > 0, "should have Python entities");

    // Verify edges were created (calls from greet→greet in Python)
    let calls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'calls'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(calls > 0, "should have calls edges");
}

#[test]
fn code_entity_types_are_is_code() {
    let storage = Storage::open_memory().unwrap();
    let code = "fn test_func() {}";
    let files = vec![(
        std::path::PathBuf::from("test.rs"),
        code.to_string(),
        Language::Rust,
    )];

    sync_code(&storage, &files);

    let conn = storage.conn();
    let entities =
        storage::query::get_entities_by_type(&conn, "function", Some("active"), 100).unwrap();
    for e in &entities {
        let etype = EntityType::parse(&e.r#type).unwrap();
        assert!(etype.is_code(), "function should be code entity");
    }
}
