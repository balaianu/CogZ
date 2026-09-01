//! NLI-based contradiction detection.
//!
//! Runs on insert when `contradiction_check` is enabled and an NLI
//! model is available. Compares a new observation or rule against
//! existing entities of the same type and flags contradictions.

use rusqlite::Connection;

use crate::config::ConsolidationConfig;
use crate::embed::{NliLabel, NliModel};
use crate::storage::crud::Entity;
use crate::storage::query::get_entities_by_type;

/// Result of a contradiction check on a newly inserted entity.
#[derive(Debug, Clone)]
pub struct ContradictionResult {
    /// True if the NLI model detected at least one contradiction.
    pub contradiction_flagged: bool,
    /// IDs of entities that contradict the new entity.
    pub contradicts_ids: Vec<String>,
}

impl ContradictionResult {
    pub fn empty() -> Self {
        Self {
            contradiction_flagged: false,
            contradicts_ids: Vec::new(),
        }
    }
}

/// Run contradiction detection against existing entities of the same
/// type. Returns an empty result if the NLI model is unavailable or
/// contradiction checking is disabled in config.
///
/// The new entity's content is the premise; each existing entity's
/// content is the hypothesis. If the NLI label is `Contradiction`,
/// the existing entity ID is recorded and `contradiction_flagged` is
/// set. The caller is responsible for recording `contradicts` edges
/// and the `contradiction_found` event.
pub fn check_contradiction(
    conn: &Connection,
    new_id: &str,
    new_content: &str,
    entity_type: &str,
    model: Option<&dyn NliModel>,
    config: &ConsolidationConfig,
) -> ContradictionResult {
    let candidates = fetch_contradiction_candidates(conn, new_id, new_content, entity_type, config);
    let contradicts_ids = classify_candidates(candidates, new_content, model);
    let contradiction_flagged = !contradicts_ids.is_empty();
    ContradictionResult {
        contradiction_flagged,
        contradicts_ids,
    }
}

/// Fetch candidate entities for contradiction checking. Returns the
/// list of existing active entities of the same type (excluding self).
/// This is the DB-only portion of contradiction detection — it should
/// be called under the storage lock, and the results passed to
/// `classify_candidates` outside the lock.
pub fn fetch_contradiction_candidates(
    conn: &Connection,
    new_id: &str,
    _new_content: &str,
    entity_type: &str,
    config: &ConsolidationConfig,
) -> Vec<Entity> {
    if !config.contradiction_check {
        return Vec::new();
    }

    match get_entities_by_type(conn, entity_type, Some("active"), 50) {
        Ok(e) => e.into_iter().filter(|e| e.id != new_id).collect(),
        Err(_) => Vec::new(),
    }
}

/// Classify candidate entities against the new content using the NLI
/// model. Returns the IDs of entities that contradict the new content.
/// This is the I/O portion — it must NOT be called while holding the
/// storage lock, since ONNX inference is blocking.
pub fn classify_candidates(
    candidates: Vec<Entity>,
    new_content: &str,
    model: Option<&dyn NliModel>,
) -> Vec<String> {
    let Some(model) = model else {
        return Vec::new();
    };

    // Don't gate on is_available() — OnnxNliModel loads lazily via
    // try_load() inside classify(). is_available() returns false until
    // the first classify() call, so gating here would skip all checks.
    let mut contradicts_ids = Vec::new();
    for entity in &candidates {
        match model.classify(new_content, &entity.content) {
            Ok(NliLabel::Contradiction) => contradicts_ids.push(entity.id.clone()),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "NLI classification failed for {} vs {}: {}",
                    new_content,
                    entity.id,
                    e
                );
            }
        }
    }
    contradicts_ids
}

/// Record `contradicts` edges from the new entity to each
/// contradicted existing entity, write them to the entity's file
/// frontmatter (file-first invariant), and record a
/// `contradiction_found` domain event.
pub fn record_contradictions(
    conn: &Connection,
    new_id: &str,
    contradicts_ids: &[String],
    file_path: &std::path::Path,
) -> Result<(), crate::storage::StorageError> {
    use crate::files::frontmatter::FmValue;
    use crate::files::{read_entity_file, write_entity_file};
    use crate::storage::edges::{Edge, insert_edge};
    use crate::storage::events::{EventType, record_event};

    // 1. Update the file frontmatter with contradicts field (file-first).
    let mut entity_file = read_entity_file(file_path).map_err(|e| {
        crate::storage::StorageError::File(format!(
            "failed to read entity file {}: {}",
            file_path.display(),
            e
        ))
    })?;
    entity_file
        .frontmatter
        .insert("contradicts", FmValue::Array(contradicts_ids.to_vec()));
    entity_file.updated_at = chrono::Utc::now().to_rfc3339();
    write_entity_file(file_path, &entity_file).map_err(|e| {
        crate::storage::StorageError::File(format!(
            "failed to write entity file {}: {}",
            file_path.display(),
            e
        ))
    })?;

    // 2. Insert contradicts edges in the DB.
    let now = chrono::Utc::now().to_rfc3339();
    for target_id in contradicts_ids {
        insert_edge(
            conn,
            &Edge {
                source_id: new_id.to_string(),
                target_id: target_id.clone(),
                edge_type: "contradicts".to_string(),
                weight: 1.0,
                created_at: now.clone(),
            },
        )?;
    }

    let payload = serde_json::json!({
        "contradicts_ids": contradicts_ids,
        "model": "nli",
    });
    record_event(conn, EventType::ContradictionFound, Some(new_id), &payload)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::MockNliModel;
    use crate::storage::crud::{Entity, insert_entity};
    use crate::storage::ensure_vec_extension;
    use crate::storage::schema::run_migrations;

    fn setup() -> Connection {
        ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();
        conn
    }

    fn config(check: bool) -> ConsolidationConfig {
        ConsolidationConfig {
            dedup_threshold: 0.92,
            title_match_threshold: 0.85,
            contradiction_check: check,
            promotion_threshold: 3,
        }
    }

    #[test]
    fn disabled_in_config_returns_empty() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "yes")).unwrap();
        let result = check_contradiction(
            &conn,
            "u2",
            "no",
            "observation",
            Some(&MockNliModel),
            &config(false),
        );
        assert!(!result.contradiction_flagged);
        assert!(result.contradicts_ids.is_empty());
    }

    #[test]
    fn no_model_returns_empty() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "yes")).unwrap();
        let result = check_contradiction(&conn, "u2", "no", "observation", None, &config(true));
        assert!(!result.contradiction_flagged);
    }

    #[test]
    fn detects_contradiction_with_mock_model() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "A", "The bug is in search"),
        )
        .unwrap();

        let result = check_contradiction(
            &conn,
            "u2",
            "The bug is not in search",
            "observation",
            Some(&MockNliModel),
            &config(true),
        );
        assert!(result.contradiction_flagged);
        assert_eq!(result.contradicts_ids, vec!["u1".to_string()]);
    }

    #[test]
    fn no_contradiction_for_entailment() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "A", "The bug is in search"),
        )
        .unwrap();

        let result = check_contradiction(
            &conn,
            "u2",
            "The bug is in search",
            "observation",
            Some(&MockNliModel),
            &config(true),
        );
        assert!(!result.contradiction_flagged);
    }

    #[test]
    fn skips_self() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "A", "The bug is in search"),
        )
        .unwrap();

        let result = check_contradiction(
            &conn,
            "u1",
            "The bug is in search",
            "observation",
            Some(&MockNliModel),
            &config(true),
        );
        assert!(!result.contradiction_flagged);
    }

    #[test]
    fn record_contradictions_creates_edges_and_event() {
        use crate::files::frontmatter::{FmValue, Frontmatter, serialize as serialize_fm};

        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u3", "observation", "C", "c")).unwrap();

        // Write a file for u3 so record_contradictions can update it.
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("u3.md");
        let mut fm = Frontmatter::new();
        fm.insert("id", FmValue::String("u3".to_string()));
        fm.insert("title", FmValue::String("C".to_string()));
        fm.insert("type", FmValue::String("observation".to_string()));
        fm.insert("status", FmValue::String("active".to_string()));
        fm.insert(
            "created_at",
            FmValue::String("2026-01-01T00:00:00Z".to_string()),
        );
        fm.insert(
            "updated_at",
            FmValue::String("2026-01-01T00:00:00Z".to_string()),
        );
        fm.insert("references", FmValue::Array(vec![]));
        std::fs::write(&file_path, format!("---\n{}---\n\nc", serialize_fm(&fm))).unwrap();

        record_contradictions(
            &conn,
            "u3",
            &["u1".to_string(), "u2".to_string()],
            &file_path,
        )
        .unwrap();

        let edges = crate::storage::edges::get_edges_from(&conn, "u3").unwrap();
        let contradict_edges: Vec<_> = edges
            .iter()
            .filter(|e| e.edge_type == "contradicts")
            .collect();
        assert_eq!(contradict_edges.len(), 2);

        let events = crate::storage::events::get_events_for_entity(&conn, "u3").unwrap();
        assert!(events.iter().any(|e| e.event_type == "contradiction_found"));

        // Verify the file was updated with contradicts frontmatter.
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("contradicts"));
    }
}
