use super::{Language, extract_entities};
use std::path::Path;

#[test]
fn extract_rust_function() {
    let code = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
    let entities = extract_entities(Path::new("src/math.rs"), code, Language::Rust);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "add");
    assert_eq!(funcs[0].properties["line_start"], 2);
    assert_eq!(funcs[0].properties["line_end"], 4);
    assert_eq!(funcs[0].properties["language"], "rust");
    assert!(
        funcs[0].properties["signature"]
            .as_str()
            .unwrap()
            .contains("fn add")
    );
}

#[test]
fn extract_rust_struct() {
    let code = r#"
pub struct Point {
    x: f64,
    y: f64,
}
"#;
    let entities = extract_entities(Path::new("src/point.rs"), code, Language::Rust);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].title, "Point");
    assert_eq!(classes[0].properties["kind"], "struct_item");
}

#[test]
fn extract_rust_impl_with_methods() {
    let code = r#"
impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn distance(&self, other: &Point) -> f64 {
        0.0
    }
}
"#;
    let entities = extract_entities(Path::new("src/point.rs"), code, Language::Rust);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].title, "impl Point");

    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 2);
    assert_eq!(funcs[0].properties["qualified_name"], "Point::new");
    assert_eq!(funcs[1].properties["qualified_name"], "Point::distance");
}

#[test]
fn extract_rust_module() {
    let code = r#"
pub mod storage {
    pub fn foo() {}
}
"#;
    let entities = extract_entities(Path::new("src/lib.rs"), code, Language::Rust);
    let mods: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "module")
        .collect();
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].title, "storage");
    assert_eq!(mods[0].properties["module_path"], "storage");
}

#[test]
fn extract_rust_file_entity_always_present() {
    let code = "fn main() {}";
    let entities = extract_entities(Path::new("src/main.rs"), code, Language::Rust);
    let files: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "file")
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].title, "main.rs");
    assert_eq!(files[0].properties["language"], "rust");
    assert_eq!(files[0].properties["line_count"], 1);
}

#[test]
fn extract_python_function() {
    let code = r#"
def add(a, b):
    return a + b
"#;
    let entities = extract_entities(Path::new("math.py"), code, Language::Python);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "add");
    assert_eq!(funcs[0].properties["line_start"], 2);
    assert_eq!(funcs[0].properties["language"], "python");
}

#[test]
fn extract_python_class_with_methods() {
    let code = r#"
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def distance(self, other):
        return 0.0
"#;
    let entities = extract_entities(Path::new("point.py"), code, Language::Python);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].title, "Point");

    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 2);
    assert_eq!(funcs[0].properties["qualified_name"], "Point::__init__");
    assert_eq!(funcs[1].properties["qualified_name"], "Point::distance");
}

#[test]
fn extract_python_class_with_superclass() {
    let code = r#"
class Dog(Animal):
    def bark(self):
        pass
"#;
    let entities = extract_entities(Path::new("dog.py"), code, Language::Python);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    let superclasses = classes[0].properties["superclasses"].as_array().unwrap();
    assert_eq!(superclasses.len(), 1);
    assert_eq!(superclasses[0], "Animal");
}

#[test]
fn extract_python_file_entity_always_present() {
    let code = "x = 1";
    let entities = extract_entities(Path::new("script.py"), code, Language::Python);
    let files: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "file")
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].title, "script.py");
    assert_eq!(files[0].properties["language"], "python");
}

#[test]
fn empty_file_produces_only_file_entity() {
    let code = "";
    let entities = extract_entities(Path::new("empty.rs"), code, Language::Rust);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].entity_type, "file");
}

#[test]
fn syntax_error_does_not_panic() {
    let code = "fn broken(";
    let entities = extract_entities(Path::new("bad.rs"), code, Language::Rust);
    assert!(!entities.is_empty());
    assert_eq!(entities[0].entity_type, "file");
}
