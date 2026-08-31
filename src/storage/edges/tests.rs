//! Tests for the edges module.

use super::super::crud::{Entity, insert_entity};
use super::super::ensure_vec_extension;
use super::super::schema::run_migrations;
use super::*;

use rusqlite::Connection;

fn setup() -> Connection {
    ensure_vec_extension();
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, 768).unwrap();
    conn
}

fn edge(source: &str, target: &str, edge_type: &str) -> Edge {
    Edge {
        source_id: source.to_string(),
        target_id: target.to_string(),
        edge_type: edge_type.to_string(),
        weight: 1.0,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[test]
fn insert_and_count_edges() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

    insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
    assert_eq!(count_edges(&conn).unwrap(), 1);
}

#[test]
fn get_edges_from_and_to() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

    insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();

    let from = get_edges_from(&conn, "u1").unwrap();
    assert_eq!(from.len(), 1);
    assert_eq!(from[0].target_id, "u2");

    let to = get_edges_to(&conn, "u2").unwrap();
    assert_eq!(to.len(), 1);
    assert_eq!(to[0].source_id, "u1");
}

#[test]
fn remove_edge() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

    insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
    assert_eq!(count_edges(&conn).unwrap(), 1);

    delete_edge(&conn, "u1", "u2", "references").unwrap();
    assert_eq!(count_edges(&conn).unwrap(), 0);
}

#[test]
fn neighbors_batch_returns_both_directions() {
    let conn = setup();
    for id in &["u1", "u2", "u3", "u4"] {
        insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
    }
    insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
    insert_edge(&conn, &edge("u3", "u1", "references")).unwrap();

    let (outgoing, incoming) = get_neighbors_batch(&conn, &["u1".to_string()]).unwrap();
    assert!(outgoing.contains(&"u2".to_string()));
    assert!(incoming.contains(&"u3".to_string()));
}

#[test]
fn neighbors_batch_multi_node() {
    let conn = setup();
    for id in &["u1", "u2", "u3", "u4", "u5"] {
        insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
    }
    insert_edge(&conn, &edge("u1", "u4", "references")).unwrap();
    insert_edge(&conn, &edge("u2", "u5", "references")).unwrap();
    insert_edge(&conn, &edge("u3", "u1", "references")).unwrap();

    let frontier = vec!["u1".to_string(), "u2".to_string()];
    let (outgoing, incoming) = get_neighbors_batch(&conn, &frontier).unwrap();
    assert!(outgoing.contains(&"u4".to_string()));
    assert!(outgoing.contains(&"u5".to_string()));
    assert!(incoming.contains(&"u3".to_string()));
}

#[test]
fn neighbors_batch_empty_frontier() {
    let conn = setup();
    let (outgoing, incoming) = get_neighbors_batch(&conn, &[]).unwrap();
    assert!(outgoing.is_empty());
    assert!(incoming.is_empty());
}

#[test]
fn edge_type_between_outgoing() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();
    insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();

    assert_eq!(
        get_edge_type_between(&conn, "u1", "u2").unwrap(),
        Some("references".to_string())
    );
}

#[test]
fn edge_type_between_incoming() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();
    insert_edge(&conn, &edge("u2", "u1", "references")).unwrap();

    assert_eq!(
        get_edge_type_between(&conn, "u1", "u2").unwrap(),
        Some("references".to_string())
    );
}

#[test]
fn edge_type_between_none() {
    let conn = setup();
    insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
    insert_entity(&conn, &Entity::new("u2", "function", "B", "c")).unwrap();

    assert_eq!(get_edge_type_between(&conn, "u1", "u2").unwrap(), None);
}

#[test]
fn delete_structural_edges_by_sources_targets_only_changed() {
    let conn = setup();
    for id in &["u1", "u2", "u3"] {
        insert_entity(&conn, &Entity::new(id, "function", id, "c")).unwrap();
    }
    // u1 calls u2 (outgoing from u1)
    insert_edge(&conn, &edge("u1", "u2", "calls")).unwrap();
    // u3 calls u1 (incoming to u1, outgoing from u3)
    insert_edge(&conn, &edge("u3", "u1", "calls")).unwrap();

    delete_structural_edges_by_sources(&conn, &["u1".to_string()]).unwrap();
    // Only u1's outgoing edges deleted; u3→u1 preserved.
    assert_eq!(count_edges(&conn).unwrap(), 1);
    let remaining = get_edges_from(&conn, "u3").unwrap();
    assert_eq!(remaining.len(), 1);
}

#[test]
fn delete_edges_for_entity_removes_both_directions() {
    let conn = setup();
    for id in &["u1", "u2", "u3"] {
        insert_entity(&conn, &Entity::new(id, "observation", id, "c")).unwrap();
    }
    // u1 → u2 (outgoing from u1)
    insert_edge(&conn, &edge("u1", "u2", "references")).unwrap();
    // u3 → u1 (incoming to u1)
    insert_edge(&conn, &edge("u3", "u1", "supports")).unwrap();

    delete_edges_for_entity(&conn, "u1").unwrap();

    // Both edges involving u1 should be gone.
    assert_eq!(count_edges(&conn).unwrap(), 0);
}
