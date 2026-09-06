use super::*;
use crate::index::sync::{sync_code_entities, sync_code_entities_incremental};
use crate::index::tree_sitter::{Language, extract_all};
use crate::storage::Storage;

/// Helper: parse source files and sync entities + edges to DB.
fn index_files(storage: &Storage, files: &[(std::path::PathBuf, String, Language)]) {
    let mut entities_by_file = Vec::new();
    let mut raw_edges_by_file = Vec::new();
    for (path, source, lang) in files {
        let (entities, raw_edges) = extract_all(path, source, *lang);
        let path_str = crate::index::path_to_string(path);
        entities_by_file.push((path_str.clone(), entities));
        raw_edges_by_file.push((path_str, raw_edges));
    }
    sync_code_entities(storage, &entities_by_file, &Default::default());
    sync_code_edges(storage, &entities_by_file, &raw_edges_by_file);
}

#[test]
fn python_extends_edge_created() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
class Animal:
    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        print("woof")
"#;
    let files = vec![(
        std::path::PathBuf::from("animals.py"),
        code.to_string(),
        Language::Python,
    )];

    index_files(&storage, &files);

    let conn = storage.conn();
    let extends_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'extends'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(extends_count, 1);
}

#[test]
fn python_calls_edge_created() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
def add(a, b):
    return a + b

def compute():
    return add(1, 2)
"#;
    let files = vec![(
        std::path::PathBuf::from("math.py"),
        code.to_string(),
        Language::Python,
    )];

    index_files(&storage, &files);

    let conn = storage.conn();
    let calls_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'calls'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        calls_count >= 1,
        "expected at least 1 calls edge, got {calls_count}"
    );
}

#[test]
fn rust_extends_edge_for_impl_trait() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
trait Display {
    fn show(&self);
}

struct Point {
    x: f64,
}

impl Display for Point {
    fn show(&self) {}
}
"#;
    let files = vec![(
        std::path::PathBuf::from("point.rs"),
        code.to_string(),
        Language::Rust,
    )];

    index_files(&storage, &files);

    let conn = storage.conn();
    let extends_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'extends'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(extends_count, 1);
}

#[test]
fn no_edges_for_empty_file() {
    let storage = Storage::open_memory().unwrap();
    let code = "";
    let files = vec![(
        std::path::PathBuf::from("empty.rs"),
        code.to_string(),
        Language::Rust,
    )];

    index_files(&storage, &files);

    let conn = storage.conn();
    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(edge_count, 0);
}

#[test]
fn reindex_removes_stale_calls_edges() {
    let storage = Storage::open_memory().unwrap();

    // V1: foo calls bar
    let code_v1 = "fn bar() {}\nfn foo() { bar(); }\n";
    let files_v1 = vec![(
        std::path::PathBuf::from("src/test.rs"),
        code_v1.to_string(),
        Language::Rust,
    )];
    index_files(&storage, &files_v1);

    let foo_id = code_entity_uuid("src/test.rs", "function", "foo");
    let bar_id = code_entity_uuid("src/test.rs", "function", "bar");
    {
        let conn = storage.conn();
        let has_foo_bar: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE source_id = ?1 AND target_id = ?2 AND edge_type = 'calls'",
                rusqlite::params![foo_id, bar_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_foo_bar, 1, "foo→bar calls edge should exist after v1");
    }

    // V2: foo no longer calls bar, calls baz instead
    let code_v2 = "fn baz() {}\nfn bar() {}\nfn foo() { baz(); }\n";
    let files_v2 = vec![(
        std::path::PathBuf::from("src/test.rs"),
        code_v2.to_string(),
        Language::Rust,
    )];
    index_files(&storage, &files_v2);

    let conn = storage.conn();
    let stale_foo_bar: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE source_id = ?1 AND target_id = ?2 AND edge_type = 'calls'",
            rusqlite::params![foo_id, bar_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        stale_foo_bar, 0,
        "stale foo→bar edge should be removed after reindex"
    );

    let baz_id = code_entity_uuid("src/test.rs", "function", "baz");
    let has_foo_baz: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE source_id = ?1 AND target_id = ?2 AND edge_type = 'calls'",
            rusqlite::params![foo_id, baz_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_foo_baz, 1, "foo→baz calls edge should exist after v2");
}

#[test]
fn reindex_preserves_references_edges() {
    let storage = Storage::open_memory().unwrap();

    // Index code
    let code = "fn bar() {}\nfn foo() { bar(); }\n";
    let files = vec![(
        std::path::PathBuf::from("src/test.rs"),
        code.to_string(),
        Language::Rust,
    )];
    index_files(&storage, &files);

    // Manually add a references edge (simulating user-created edge from frontmatter)
    let bar_id = code_entity_uuid("src/test.rs", "function", "bar");
    {
        let conn = storage.conn();
        // Insert a dummy observation entity for the FK constraint
        storage::crud::insert_entity(
            &conn,
            &storage::crud::Entity {
                id: "manual-obs-id".to_string(),
                r#type: "observation".to_string(),
                title: Some("test obs".to_string()),
                content: "test".to_string(),
                properties: serde_json::json!({}),
                file_path: None,
                status: "active".to_string(),
                content_hash: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        storage::edges::insert_edge(
            &conn,
            &storage::edges::Edge {
                source_id: "manual-obs-id".to_string(),
                target_id: bar_id.clone(),
                edge_type: "references".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    // Reindex code edges
    let mut entities_by_file = Vec::new();
    let mut raw_edges_by_file = Vec::new();
    for (path, source, lang) in &files {
        let (entities, raw_edges) = extract_all(path, source, *lang);
        let path_str = crate::index::path_to_string(path);
        entities_by_file.push((path_str.clone(), entities));
        raw_edges_by_file.push((path_str, raw_edges));
    }
    sync_code_edges(&storage, &entities_by_file, &raw_edges_by_file);

    // references edge should still exist
    let conn = storage.conn();
    let refs_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'references'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        refs_count, 1,
        "references edges must be preserved across code reindex"
    );
}

#[test]
fn contains_edges_link_files_to_functions_and_classes() {
    let storage = Storage::open_memory().unwrap();
    let code = r#"
fn helper() -> u32 { 42 }

struct Point { x: f64, y: f64 }

impl Point {
    fn new(x: f64, y: f64) -> Self { Point { x, y } }
}
"#;
    let files = vec![(
        std::path::PathBuf::from("geometry.rs"),
        code.to_string(),
        Language::Rust,
    )];

    index_files(&storage, &files);

    let conn = storage.conn();
    let contains_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'contains'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // helper function, Point struct (class), Point::new method (function)
    assert!(
        contains_count >= 2,
        "expected at least 2 contains edges, got {contains_count}"
    );

    // All contains edges should originate from a file entity
    let non_file_sources: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN entities src ON e.source_id = src.id
             WHERE e.edge_type = 'contains' AND src.type != 'file'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(non_file_sources, 0, "contains edges must come from files");
}

#[test]
fn incremental_reindex_preserves_import_edges() {
    let storage = Storage::open_memory().unwrap();

    let utils_code = "pub fn helper() {}\n";
    let main_code = "use utils::helper;\nfn main() { helper(); }\n";

    let files = vec![
        (
            std::path::PathBuf::from("src/utils.rs"),
            utils_code.to_string(),
            Language::Rust,
        ),
        (
            std::path::PathBuf::from("src/main.rs"),
            main_code.to_string(),
            Language::Rust,
        ),
    ];

    index_files(&storage, &files);

    let imports_after_full: i64 = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'imports'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        imports_after_full >= 1,
        "expected at least 1 imports edge after full index, got {imports_after_full}"
    );

    // Incremental reindex: re-parse main.rs only (the importing file).
    let changed = vec![(
        std::path::PathBuf::from("src/main.rs"),
        main_code.to_string(),
        Language::Rust,
    )];

    let mut entities_by_file = Vec::new();
    for (path, source, lang) in &changed {
        let (entities, _) = extract_all(path, source, *lang);
        entities_by_file.push((crate::index::path_to_string(path), entities));
    }
    sync_code_entities_incremental(&storage, &entities_by_file);
    sync_code_edges_incremental(&storage, &changed);

    let imports_after_incremental: i64 = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'imports'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        imports_after_incremental >= 1,
        "import edges should survive incremental reindex, got {imports_after_incremental}"
    );
}
