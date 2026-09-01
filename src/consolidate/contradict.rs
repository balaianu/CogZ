//! NLI-based contradiction detection.
//!
//! Runs on insert when `contradiction_check` is enabled and an NLI
//! model is available. Compares a new observation or rule against
//! existing entities of the same type and flags contradictions.

use rusqlite::Connection;

use crate::config::ConsolidationConfig;
use crate::embed::{EmbeddingModel, NliModel};
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
/// Uses bidirectional NLI scoring (both premise→hypothesis and
/// hypothesis→premise), taking the max P(contradiction) across
/// directions. Real contradictions can score asymmetrically.
///
/// Pre-filters reduce false positives:
/// - Length ratio: texts differing by >5:1 are different content types.
/// - Cosine similarity (when an embedding model is provided): genuine
///   contradictions share the same topic, so embeddings should be
///   highly similar (≥ 0.85).
///
/// The caller is responsible for recording `contradicts` edges
/// and the `contradiction_found` event.
pub fn check_contradiction(
    conn: &Connection,
    new_id: &str,
    new_content: &str,
    entity_type: &str,
    model: Option<&dyn NliModel>,
    embed_model: Option<&dyn EmbeddingModel>,
    config: &ConsolidationConfig,
) -> ContradictionResult {
    let candidates = fetch_contradiction_candidates(conn, new_id, new_content, entity_type, config);
    let contradicts_ids = classify_candidates(candidates, new_content, model, embed_model, config);
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
///
/// Uses bidirectional scoring: runs NLI in both directions (new→existing
/// and existing→new), takes the max P(contradiction). Real contradictions
/// can score asymmetrically (benched 0.44 one direction, 0.99 the other).
///
/// Pre-filters reduce false positives before NLI inference:
/// - Identical text fast-path: skip NLI entirely.
/// - Length ratio: texts differing by > `contradiction_length_ratio`
///   are different content types, not contradictions.
/// - Cosine similarity (when an embedding model is provided): genuine
///   contradictions share the same topic, so embeddings should be
///   highly similar (≥ `contradiction_cosine_threshold`).
///
/// A pair is flagged as contradicting only when:
/// 1. Texts are not identical.
/// 2. Length ratio is within bounds.
/// 3. Cosine similarity ≥ threshold (when embedding model is available).
/// 4. Max-direction P(contradiction) ≥ `contradiction_threshold`.
pub fn classify_candidates(
    candidates: Vec<Entity>,
    new_content: &str,
    model: Option<&dyn NliModel>,
    embed_model: Option<&dyn EmbeddingModel>,
    config: &ConsolidationConfig,
) -> Vec<String> {
    let Some(model) = model else {
        return Vec::new();
    };

    let new_lower = new_content.to_lowercase();
    let new_len = new_content.len();

    // Pre-compute the new content's embedding if an embedding model is
    // available, for cosine similarity pre-filtering.
    let new_embedding = if let Some(em) = embed_model {
        em.embed(&[new_content])
            .ok()
            .and_then(|v| v.into_iter().next())
    } else {
        None
    };

    let mut contradicts_ids = Vec::new();
    for entity in &candidates {
        // Fast-path: identical texts are entailment, not contradiction.
        if new_lower == entity.content.to_lowercase() {
            continue;
        }

        // Length ratio pre-filter.
        let entity_len = entity.content.len();
        if new_len > 0 && entity_len > 0 {
            let ratio = (new_len.max(entity_len) as f64) / (new_len.min(entity_len) as f64);
            if ratio > config.contradiction_length_ratio {
                continue;
            }
        }

        // Cosine similarity pre-filter (when embedding model is available).
        if let (Some(new_emb), Some(em)) = (&new_embedding, embed_model)
            && let Ok(entity_embs) = em.embed(&[entity.content.as_str()])
            && let Some(entity_emb) = entity_embs.into_iter().next()
        {
            let cos = cosine_similarity(new_emb, &entity_emb);
            if cos < config.contradiction_cosine_threshold {
                continue;
            }
        }

        // Bidirectional NLI scoring: max P(contradiction) across both
        // directions. Real contradictions can score asymmetrically.
        let forward = model.classify(new_content, &entity.content);
        let reverse = model.classify(&entity.content, new_content);

        let max_contra = match (forward, reverse) {
            (Ok(f), Ok(r)) => f.contradiction.max(r.contradiction),
            (Ok(f), Err(_)) => f.contradiction,
            (Err(_), Ok(r)) => r.contradiction,
            (Err(e), _) => {
                tracing::warn!(
                    "NLI classification failed for {} vs {}: {}",
                    new_content,
                    entity.id,
                    e
                );
                continue;
            }
        };

        if max_contra >= config.contradiction_threshold as f32 {
            contradicts_ids.push(entity.id.clone());
        }
    }
    contradicts_ids
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
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
            contradiction_threshold: 0.70,
            contradiction_cosine_threshold: 0.85,
            contradiction_length_ratio: 5.0,
            dedup_nli_threshold: 0.85,
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
            None,
            &config(false),
        );
        assert!(!result.contradiction_flagged);
        assert!(result.contradicts_ids.is_empty());
    }

    #[test]
    fn no_model_returns_empty() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "yes")).unwrap();
        let result =
            check_contradiction(&conn, "u2", "no", "observation", None, None, &config(true));
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
            None,
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
            None,
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
            None,
            &config(true),
        );
        assert!(!result.contradiction_flagged);
    }

    #[test]
    fn length_ratio_filter_skips_uneven_pairs() {
        let conn = setup();
        let long_text = "This is a very long observation that goes on and on \
                         about many different topics in great detail, covering \
                         architecture, design patterns, testing strategies, and \
                         deployment considerations for the project.";
        insert_entity(&conn, &Entity::new("u1", "observation", "A", long_text)).unwrap();

        // "not" in a 4-word text vs a 50+ word text — length ratio > 5:1.
        let result = check_contradiction(
            &conn,
            "u2",
            "This is not relevant",
            "observation",
            Some(&MockNliModel),
            None,
            &config(true),
        );
        assert!(!result.contradiction_flagged);
    }

    #[test]
    fn identical_text_fast_path_skips_nli() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "A", "The bug is in search"),
        )
        .unwrap();

        // Same text — should be entailment, not contradiction.
        let result = check_contradiction(
            &conn,
            "u2",
            "The bug is in search",
            "observation",
            Some(&MockNliModel),
            None,
            &config(true),
        );
        assert!(!result.contradiction_flagged);
    }

    #[test]
    fn low_contradiction_probability_not_flagged() {
        let conn = setup();
        // Two unrelated texts — mock returns neutral (0.85 neutral, 0.05 contra).
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "A", "The config file is missing"),
        )
        .unwrap();

        let result = check_contradiction(
            &conn,
            "u2",
            "The bug is in search",
            "observation",
            Some(&MockNliModel),
            None,
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
