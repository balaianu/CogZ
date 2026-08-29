use super::*;
use crate::index::sync::sync_code_entities;
use crate::storage::Storage;

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

    sync_code_entities(&storage, Path::new("."), &files);
    sync_code_edges(&storage, Path::new("."), &files);

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

    sync_code_entities(&storage, Path::new("."), &files);
    sync_code_edges(&storage, Path::new("."), &files);

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

    sync_code_entities(&storage, Path::new("."), &files);
    sync_code_edges(&storage, Path::new("."), &files);

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

    sync_code_entities(&storage, Path::new("."), &files);
    sync_code_edges(&storage, Path::new("."), &files);

    let conn = storage.conn();
    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(edge_count, 0);
}
