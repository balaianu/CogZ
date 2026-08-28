//! Integration tests for context assembly.

use cogz::config::Config;
use cogz::context::{AssembleParams, ContextMode, assemble_context};
use cogz::storage::Storage;
use cogz::storage::crud::{Entity, insert_entity};
use cogz::storage::edges::{Edge, insert_edge};

fn default_config() -> Config {
    Config::default_for("test")
}

fn edge(source: &str, target: &str) -> Edge {
    Edge {
        source_id: source.to_string(),
        target_id: target.to_string(),
        edge_type: "references".to_string(),
        weight: 1.0,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[test]
fn cold_start_produces_compact_pack() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new("r1", "rule", "Rule A", "rule content a"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("r2", "rule", "Rule B", "rule content b"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("o1", "observation", "Obs A", "obs content a"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("k1", "knowledge", "Knowledge A", "knowledge content"),
    )
    .unwrap();

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    assert_eq!(pack.mode, ContextMode::ColdStart);
    // Identity + 2 rules + 1 observation = 4 sections
    assert_eq!(pack.sections.len(), 4);
    assert!(pack.sections.iter().any(|s| s.source == "identity"));
    assert!(pack.sections.iter().any(|s| s.source == "rule"));
    assert!(pack.sections.iter().any(|s| s.source == "observation"));
    assert!(!pack.sections.iter().any(|s| s.source == "knowledge"));
    assert_eq!(pack.query, "");
}

#[test]
fn task_mode_produces_query_scoped_pack() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new(
            "o1",
            "observation",
            "FTS5 ranking bug",
            "ranking bug content",
        ),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new(
            "r1",
            "rule",
            "Use parameterized queries",
            "always parameterize",
        ),
    )
    .unwrap();

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::Task,
        query: Some("ranking"),
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    assert_eq!(pack.mode, ContextMode::Task);
    assert_eq!(pack.query, "ranking");
    assert!(!pack.sections.is_empty());
    assert!(pack.sections.iter().any(|s| s.entity_id == "o1"));
}

#[test]
fn task_mode_includes_graph_paths() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    insert_entity(
        &conn,
        &Entity::new("o1", "observation", "FTS5 bug", "ranking bug content"),
    )
    .unwrap();
    insert_entity(
        &conn,
        &Entity::new("k1", "knowledge", "Search design", "how search works"),
    )
    .unwrap();
    insert_edge(&conn, &edge("o1", "k1")).unwrap();

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::Task,
        query: Some("ranking"),
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    let o1 = pack.sections.iter().find(|s| s.entity_id == "o1");
    assert!(o1.is_some());
    assert_eq!(o1.unwrap().graph_path, vec!["o1"]);

    let k1 = pack.sections.iter().find(|s| s.entity_id == "k1");
    assert!(k1.is_some());
    assert_eq!(k1.unwrap().graph_path, vec!["o1", "k1"]);
}

#[test]
fn escalation_produces_wider_pack() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    for i in 0..15 {
        insert_entity(
            &conn,
            &Entity::new(
                &format!("o{i}"),
                "observation",
                &format!("Item {i}"),
                "ranking content",
            ),
        )
        .unwrap();
    }

    let config = default_config();
    let task_params = AssembleParams {
        mode: ContextMode::Task,
        query: Some("ranking"),
        ..Default::default()
    };
    let esc_params = AssembleParams {
        mode: ContextMode::Escalation,
        query: Some("ranking"),
        ..Default::default()
    };
    let task_pack = assemble_context(&conn, &task_params, &config).unwrap();
    let esc_pack = assemble_context(&conn, &esc_params, &config).unwrap();

    assert!(esc_pack.sections.len() >= task_pack.sections.len());
}

#[test]
fn token_budget_is_respected() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    let big_content = "x".repeat(2000);
    for i in 0..10 {
        insert_entity(
            &conn,
            &Entity::new(&format!("r{i}"), "rule", &format!("Rule {i}"), &big_content),
        )
        .unwrap();
    }

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        max_tokens: Some(100),
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    assert!(pack.metadata.size_tokens <= 100);
    assert!(!pack.metadata.dropped_sources.is_empty());
}

#[test]
fn dropped_sources_are_listed() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    let big_content = "x".repeat(2000);
    insert_entity(&conn, &Entity::new("r1", "rule", "R1", &big_content)).unwrap();
    insert_entity(&conn, &Entity::new("r2", "rule", "R2", &big_content)).unwrap();

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        max_tokens: Some(50),
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    assert!(!pack.metadata.dropped_sources.is_empty());
    for src in &pack.metadata.dropped_sources {
        assert!(src.contains("over token budget"));
    }
}

#[test]
fn task_mode_without_query_returns_error() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::Task,
        query: None,
        ..Default::default()
    };
    let result = assemble_context(&conn, &params, &config);
    assert!(result.is_err());
}

#[test]
fn cold_start_respects_config_limits() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    for i in 0..10 {
        insert_entity(
            &conn,
            &Entity::new(&format!("r{i}"), "rule", &format!("Rule {i}"), "content"),
        )
        .unwrap();
    }

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    let rule_count = pack.sections.iter().filter(|s| s.source == "rule").count();
    assert_eq!(rule_count, 5);
}

#[test]
fn include_stale_includes_stale_entities() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    let mut e1 = Entity::new("o1", "observation", "Stale obs", "content");
    e1.status = "stale".to_string();
    insert_entity(&conn, &e1).unwrap();
    insert_entity(
        &conn,
        &Entity::new("o2", "observation", "Active obs", "content"),
    )
    .unwrap();

    let config = default_config();

    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();
    assert!(!pack.sections.iter().any(|s| s.entity_id == "o1"));

    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        include_stale: true,
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();
    assert!(pack.sections.iter().any(|s| s.entity_id == "o1"));
}

#[test]
fn fts_only_search_mode_in_metadata() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    insert_entity(&conn, &Entity::new("r1", "rule", "Rule", "content")).unwrap();

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    assert_eq!(pack.metadata.search_mode, "fts_only");
}

#[test]
fn selected_sources_lists_unique_source_types() {
    let storage = Storage::open_memory().unwrap();
    let conn = storage.conn();
    insert_entity(&conn, &Entity::new("r1", "rule", "R1", "content")).unwrap();
    insert_entity(&conn, &Entity::new("r2", "rule", "R2", "content")).unwrap();

    let config = default_config();
    let params = AssembleParams {
        mode: ContextMode::ColdStart,
        ..Default::default()
    };
    let pack = assemble_context(&conn, &params, &config).unwrap();

    // selected_sources contains unique source types, not per-section entries.
    // With 2 rules + identity, sections = 3 but source types = 2 (identity + rule).
    assert!(
        pack.metadata
            .selected_sources
            .contains(&"identity".to_string())
    );
    assert!(pack.metadata.selected_sources.contains(&"rule".to_string()));
    assert_eq!(
        pack.metadata.selected_sources.len(),
        2,
        "expected deduplicated source types, got {:?}",
        pack.metadata.selected_sources
    );
}
