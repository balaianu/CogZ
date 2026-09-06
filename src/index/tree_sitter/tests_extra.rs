use super::{Language, Path, extract_entities};

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
