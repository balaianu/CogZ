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

// ── Go tests ───────────────────────────────────────────────────────

#[test]
fn extract_go_function() {
    let code = r#"
package main

func add(a, b int) int {
    return a + b
}
"#;
    let entities = extract_entities(Path::new("math.go"), code, Language::Go);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "add");
    assert_eq!(funcs[0].properties["language"], "go");
    assert_eq!(funcs[0].properties["qualified_name"], "add");
}

#[test]
fn extract_go_method() {
    let code = r#"
package main

type Calculator struct{}

func (c Calculator) Multiply(a, b int) int {
    return a * b
}
"#;
    let entities = extract_entities(Path::new("calc.go"), code, Language::Go);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "Calculator::Multiply");
    assert_eq!(
        funcs[0].properties["qualified_name"],
        "Calculator::Multiply"
    );
    assert_eq!(funcs[0].properties["receiver_type"], "Calculator");
}

#[test]
fn extract_go_struct_and_interface() {
    let code = r#"
package main

type Server struct {
    Addr string
}

type Handler interface {
    Serve() error
}
"#;
    let entities = extract_entities(Path::new("types.go"), code, Language::Go);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 2);
    assert!(
        classes
            .iter()
            .any(|c| c.title == "Server" && c.properties["kind"] == "struct")
    );
    assert!(
        classes
            .iter()
            .any(|c| c.title == "Handler" && c.properties["kind"] == "interface")
    );
}

#[test]
fn extract_go_package_as_module() {
    let code = r#"
package mypackage

func foo() {}
"#;
    let entities = extract_entities(Path::new("pkg.go"), code, Language::Go);
    let modules: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "module")
        .collect();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].title, "mypackage");
}

#[test]
fn extract_go_file_entity_always_present() {
    let code = "package main\n";
    let entities = extract_entities(Path::new("main.go"), code, Language::Go);
    let files: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "file")
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].title, "main.go");
    assert_eq!(files[0].properties["language"], "go");
}

// ── JavaScript tests ───────────────────────────────────────────────

#[test]
fn extract_js_function() {
    let code = r#"
function add(a, b) {
    return a + b;
}
"#;
    let entities = extract_entities(Path::new("math.js"), code, Language::JavaScript);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "add");
    assert_eq!(funcs[0].properties["language"], "javascript");
}

#[test]
fn extract_js_arrow_function() {
    let code = r#"
const multiply = (a, b) => a * b;
"#;
    let entities = extract_entities(Path::new("arrow.js"), code, Language::JavaScript);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "multiply");
    assert_eq!(funcs[0].properties["kind"], "arrow_function");
}

#[test]
fn extract_js_class_with_methods() {
    let code = r#"
class Calculator {
    add(a, b) {
        return a + b;
    }
    static create() {
        return new Calculator();
    }
}
"#;
    let entities = extract_entities(Path::new("calc.js"), code, Language::JavaScript);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].title, "Calculator");

    let methods: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().any(|m| m.title == "Calculator::add"));
    assert!(methods.iter().any(|m| m.title == "Calculator::create"));
}

#[test]
fn extract_js_class_extends() {
    let code = r#"
class Dog extends Animal {
    bark() {}
}
"#;
    let entities = extract_entities(Path::new("dog.js"), code, Language::JavaScript);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].properties["superclass"], "Animal");
}

#[test]
fn extract_js_exported_function() {
    let code = r#"
export function handler(req, res) {
    return res.json({});
}
"#;
    let entities = extract_entities(Path::new("handler.js"), code, Language::JavaScript);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "handler");
}

#[test]
fn extract_js_file_entity_always_present() {
    let code = "const x = 1;";
    let entities = extract_entities(Path::new("script.js"), code, Language::JavaScript);
    let files: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "file")
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].title, "script.js");
    assert_eq!(files[0].properties["language"], "javascript");
}

// ── TypeScript tests ───────────────────────────────────────────────

#[test]
fn extract_ts_function_with_types() {
    let code = r#"
function add(a: number, b: number): number {
    return a + b;
}
"#;
    let entities = extract_entities(Path::new("math.ts"), code, Language::TypeScript);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "add");
    assert_eq!(funcs[0].properties["language"], "typescript");
}

#[test]
fn extract_ts_interface() {
    let code = r#"
interface User {
    id: number;
    name: string;
}
"#;
    let entities = extract_entities(Path::new("user.ts"), code, Language::TypeScript);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].title, "User");
    assert_eq!(classes[0].properties["kind"], "interface");
}

#[test]
fn extract_ts_enum() {
    let code = r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#;
    let entities = extract_entities(Path::new("color.ts"), code, Language::TypeScript);
    let classes: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "class")
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].title, "Color");
    assert_eq!(classes[0].properties["kind"], "enum");
}

#[test]
fn extract_tsx_component() {
    let code = r#"
function Button(props: { label: string }) {
    return <button>{props.label}</button>;
}
"#;
    let entities = extract_entities(Path::new("Button.tsx"), code, Language::Tsx);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "Button");
    assert_eq!(funcs[0].properties["language"], "tsx");
}

// ── Bash tests ─────────────────────────────────────────────────────

#[test]
fn extract_bash_function() {
    let code = r#"
#!/bin/bash
build_project() {
    echo "Building"
    make all
}
"#;
    let entities = extract_entities(Path::new("build.sh"), code, Language::Bash);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].title, "build_project");
    assert_eq!(funcs[0].properties["language"], "bash");
    assert_eq!(funcs[0].properties["qualified_name"], "build_project");
}

#[test]
fn extract_bash_multiple_functions() {
    let code = r#"
#!/bin/bash
foo() {
    echo "foo"
}

bar() {
    foo
    ls -la
}
"#;
    let entities = extract_entities(Path::new("script.sh"), code, Language::Bash);
    let funcs: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "function")
        .collect();
    assert_eq!(funcs.len(), 2);
    assert!(funcs.iter().any(|f| f.title == "foo"));
    assert!(funcs.iter().any(|f| f.title == "bar"));
}

#[test]
fn extract_bash_file_entity_always_present() {
    let code = "#!/bin/bash\necho hello\n";
    let entities = extract_entities(Path::new("run.sh"), code, Language::Bash);
    let files: Vec<_> = entities
        .iter()
        .filter(|e| e.entity_type == "file")
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].title, "run.sh");
    assert_eq!(files[0].properties["language"], "bash");
}
