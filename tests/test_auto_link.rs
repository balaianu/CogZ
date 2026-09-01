//! Integration tests for auto-linking knowledge entities to code entities.

use std::path::Path;

use cogz::index::{self, tree_sitter::Language};
use cogz::storage::{self, Storage, crud::Entity};

fn setup_storage() -> Storage {
    Storage::open_memory().unwrap()
}

#[test]
fn auto_links_knowledge_to_code_by_path() {
    let storage = setup_storage();

    // Index a source file
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

    index::sync::sync_code_entities(&storage, Path::new("."), &files);
    index::code_graph::sync_code_edges(&storage, Path::new("."), &files);

    // Insert a knowledge entity that references the file path
    let conn = storage.conn();
    let entity = Entity::new(
        "k-001",
        "knowledge",
        "SQL builder design",
        "The query builder in src/query.rs constructs SQL strings.",
    );
    storage::crud::insert_entity(&conn, &entity).unwrap();
    drop(conn);

    // Run auto-linking
    let link_count = index::auto_link::sync_auto_links(&storage);
    assert!(link_count > 0, "should have created auto-link edges");

    // Verify the edge exists
    let conn = storage.conn();
    let auto_ref_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'auto_references'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(auto_ref_count > 0, "auto_references edges should exist");
}

#[test]
fn auto_links_knowledge_to_code_by_name() {
    let storage = setup_storage();

    let code = r#"
fn compute_embedding_similarity(a: &[f32], b: &[f32]) -> f32 {
    0.0
}
"#;
    let files = vec![(
        std::path::PathBuf::from("src/similarity.rs"),
        code.to_string(),
        Language::Rust,
    )];

    index::sync::sync_code_entities(&storage, Path::new("."), &files);
    index::code_graph::sync_code_edges(&storage, Path::new("."), &files);

    // Knowledge entity that mentions the function name
    let conn = storage.conn();
    let entity = Entity::new(
        "k-002",
        "knowledge",
        "Embedding similarity",
        "We use compute_embedding_similarity to compare vectors.",
    );
    storage::crud::insert_entity(&conn, &entity).unwrap();
    drop(conn);

    let link_count = index::auto_link::sync_auto_links(&storage);
    assert!(link_count > 0, "should link by function name");
}

#[test]
fn auto_links_are_rebuildable() {
    let storage = setup_storage();

    let code = r#"fn unique_function_name() {}"#;
    let files = vec![(
        std::path::PathBuf::from("src/mod.rs"),
        code.to_string(),
        Language::Rust,
    )];

    index::sync::sync_code_entities(&storage, Path::new("."), &files);
    index::code_graph::sync_code_edges(&storage, Path::new("."), &files);

    let conn = storage.conn();
    let entity = Entity::new(
        "k-003",
        "knowledge",
        "Test",
        "Calls unique_function_name somewhere.",
    );
    storage::crud::insert_entity(&conn, &entity).unwrap();
    drop(conn);

    // First run
    let count1 = index::auto_link::sync_auto_links(&storage);
    assert!(count1 > 0);

    // Second run — should not duplicate
    let count2 = index::auto_link::sync_auto_links(&storage);
    assert_eq!(count2, count1, "re-running should not duplicate edges");

    let conn = storage.conn();
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'auto_references'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(total as usize, count1, "no duplicate edges after rebuild");
}

#[test]
fn auto_links_preserve_manual_references() {
    let storage = setup_storage();

    let code = r#"fn some_function() {}"#;
    let files = vec![(
        std::path::PathBuf::from("src/lib.rs"),
        code.to_string(),
        Language::Rust,
    )];

    index::sync::sync_code_entities(&storage, Path::new("."), &files);
    index::code_graph::sync_code_edges(&storage, Path::new("."), &files);

    // Insert a manual references edge
    let conn = storage.conn();
    let obs = Entity::new("obs-1", "observation", "Note", "Mentions some_function.");
    storage::crud::insert_entity(&conn, &obs).unwrap();

    // Get the function entity ID
    let func_id: String = conn
        .query_row(
            "SELECT id FROM entities WHERE type = 'function' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    storage::edges::insert_edge(
        &conn,
        &storage::edges::Edge {
            source_id: "obs-1".to_string(),
            target_id: func_id.clone(),
            edge_type: "references".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    drop(conn);

    // Run auto-linking
    index::auto_link::sync_auto_links(&storage);

    // Manual references should still be there
    let conn = storage.conn();
    let manual_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE edge_type = 'references'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(manual_count, 1, "manual references must be preserved");
}
