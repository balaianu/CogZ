//! Observation-to-rule promotion.
//!
//! An observation with enough supporting observations (count ≥
//! `promotion_threshold`) is promoted to a rule. Promotion creates a
//! new rule file with a `derived_from` edge to the source observation
//! and `supporting_ids` listing all supporters. The source observation
//! is not mutated — promotion is additive.

use std::path::Path;

use crate::config::ConsolidationConfig;
use crate::files::frontmatter::{FmValue, Frontmatter};
use crate::files::{EntityFile, FileEntityType, write_entity_file};
use crate::storage::StorageError;
use crate::storage::crud::Entity;
use crate::storage::edges::{Edge, insert_edge};
use crate::storage::events::{EventType, record_event};
use crate::storage::query::get_entities_by_type;

/// Result of a single promotion decision.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PromotionResult {
    pub observation_id: String,
    pub new_rule_id: String,
    pub reason: String,
}

/// Run promotion across all active observations. Returns the list of
/// promoted observations. When `dry_run` is true, no files or DB
/// changes are made — only the candidates are reported.
pub fn run_promotion(
    storage: &crate::storage::Storage,
    cogz_dir: &Path,
    config: &ConsolidationConfig,
    dry_run: bool,
) -> Result<Vec<PromotionResult>, StorageError> {
    let candidates = find_promotion_candidates(storage, config)?;

    if dry_run {
        return Ok(candidates
            .into_iter()
            .map(|c| PromotionResult {
                observation_id: c.observation.id,
                new_rule_id: String::new(),
                reason: format!("{} supporting observations", c.supporting_ids.len()),
            })
            .collect());
    }

    let mut results = Vec::new();
    for candidate in candidates {
        match promote_one(storage, cogz_dir, &candidate) {
            Ok(rule_id) => results.push(PromotionResult {
                observation_id: candidate.observation.id,
                new_rule_id: rule_id.clone(),
                reason: format!("{} supporting observations", candidate.supporting_ids.len()),
            }),
            Err(e) => {
                tracing::warn!(
                    "promotion failed for observation {}: {}",
                    candidate.observation.id,
                    e
                );
            }
        }
    }
    Ok(results)
}

/// A promotion candidate — an observation with enough supporters.
struct PromotionCandidate {
    observation: Entity,
    supporting_ids: Vec<String>,
}

/// Find observations that meet the promotion threshold. An observation
/// is a candidate if it has ≥ `promotion_threshold` supporting
/// observations linked via `supports` edges pointing to it.
fn find_promotion_candidates(
    storage: &crate::storage::Storage,
    config: &ConsolidationConfig,
) -> Result<Vec<PromotionCandidate>, StorageError> {
    use crate::storage::edges::get_edges_to;

    let observations = {
        let conn = storage.conn();
        get_entities_by_type(&conn, "observation", Some("active"), 500)?
    };

    let mut candidates = Vec::new();
    for obs in &observations {
        let conn = storage.conn();
        let edges = get_edges_to(&conn, &obs.id)?;
        let supporting_ids: Vec<String> = edges
            .iter()
            .filter(|e| e.edge_type == "supports")
            .map(|e| e.source_id.clone())
            .collect();
        if supporting_ids.len() >= config.promotion_threshold as usize {
            candidates.push(PromotionCandidate {
                observation: obs.clone(),
                supporting_ids,
            });
        }
    }
    Ok(candidates)
}

/// Promote a single observation to a rule. Writes the rule file first,
/// then syncs the DB: inserts the rule entity, creates a `derived_from`
/// edge from the new rule to the source observation, and records a
/// `rule_promoted` event.
fn promote_one(
    storage: &crate::storage::Storage,
    cogz_dir: &Path,
    candidate: &PromotionCandidate,
) -> Result<String, StorageError> {
    use crate::files::content_hash;

    let title = candidate
        .observation
        .title
        .as_deref()
        .unwrap_or("Promoted rule");

    // Build the rule file with provenance frontmatter.
    let mut fm = Frontmatter::new();
    fm.insert("confidence", FmValue::Float(0.7));
    fm.insert("validation_count", FmValue::Int(0));
    fm.insert(
        "supporting_ids",
        FmValue::Array(candidate.supporting_ids.clone()),
    );
    fm.insert(
        "promoted_from",
        FmValue::String(candidate.observation.id.clone()),
    );
    // derived_from as frontmatter field — syncs to derived_from edge
    // via refs::sync_references, surviving DB rebuilds.
    fm.insert(
        "derived_from",
        FmValue::String(candidate.observation.id.clone()),
    );

    let mut rule_file =
        EntityFile::new(title, FileEntityType::Rule, &candidate.observation.content);
    rule_file.frontmatter = fm;

    // Write the file first (file-first invariant).
    let file_path = rule_file.file_path(cogz_dir);
    write_entity_file(&file_path, &rule_file)
        .map_err(|e| StorageError::File(format!("{}: {}", file_path.display(), e)))?;

    let rule_id = rule_file.id.clone();
    let now = chrono::Utc::now().to_rfc3339();

    // Compute content hash from the file content (same as sync.rs).
    let file_content = rule_file.to_file_content();
    let hash = content_hash(&file_content);

    // Path relative to .cogz, matching sync.rs convention.
    let relative_path = file_path
        .strip_prefix(cogz_dir)
        .unwrap_or(&file_path)
        .to_string_lossy()
        .to_string();

    // Sync to DB: insert the rule entity.
    let conn = storage.conn();
    let entity = Entity {
        id: rule_id.clone(),
        r#type: "rule".to_string(),
        title: Some(title.to_string()),
        content: candidate.observation.content.clone(),
        properties: serde_json::json!({
            "confidence": 0.7,
            "validation_count": 0,
            "supporting_ids": candidate.supporting_ids,
            "promoted_from": candidate.observation.id,
        }),
        file_path: Some(relative_path),
        status: "active".to_string(),
        content_hash: Some(hash),
        created_at: now.clone(),
        updated_at: now,
    };
    crate::storage::crud::insert_entity(&conn, &entity)?;

    // Create derived_from edge: new rule → source observation.
    // This edge is also file-backed via the derived_from frontmatter
    // field, so it survives DB rebuilds.
    insert_edge(
        &conn,
        &Edge {
            source_id: rule_id.clone(),
            target_id: candidate.observation.id.clone(),
            edge_type: "derived_from".to_string(),
            weight: 1.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;

    // Record the promotion event.
    let payload = serde_json::json!({
        "observation_id": candidate.observation.id,
        "supporting_ids": candidate.supporting_ids,
    });
    record_event(&conn, EventType::RulePromoted, Some(&rule_id), &payload)?;

    Ok(rule_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::crud::{Entity, insert_entity};
    use crate::storage::edges::{Edge, insert_edge};
    use crate::storage::ensure_vec_extension;
    use rusqlite::Connection;

    fn setup() -> crate::storage::Storage {
        ensure_vec_extension();
        crate::storage::Storage::open_memory().unwrap()
    }

    fn config(threshold: u32) -> ConsolidationConfig {
        ConsolidationConfig {
            dedup_threshold: 0.92,
            title_match_threshold: 0.85,
            contradiction_check: true,
            promotion_threshold: threshold,
            contradiction_threshold: 0.70,
            contradiction_cosine_threshold: 0.85,
            contradiction_length_ratio: 5.0,
            dedup_nli_threshold: 0.85,
        }
    }

    fn add_supports_edge(conn: &Connection, source: &str, target: &str) {
        insert_edge(
            conn,
            &Edge {
                source_id: source.to_string(),
                target_id: target.to_string(),
                edge_type: "supports".to_string(),
                weight: 1.0,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    #[test]
    fn dry_run_reports_candidates_without_changes() {
        let storage = setup();
        let dir = tempfile::tempdir().unwrap();
        let cogz_dir = dir.path().join(".cogz");
        std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

        {
            let conn = storage.conn();
            insert_entity(
                &conn,
                &Entity::new("obs-1", "observation", "Obs 1", "content"),
            )
            .unwrap();
            for i in 2..=4 {
                insert_entity(
                    &conn,
                    &Entity::new(&format!("sup-{i}"), "observation", &format!("Sup {i}"), "c"),
                )
                .unwrap();
                add_supports_edge(&conn, &format!("sup-{i}"), "obs-1");
            }
        }

        let results = run_promotion(&storage, &cogz_dir, &config(3), true).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].observation_id, "obs-1");
        assert!(results[0].new_rule_id.is_empty());

        // No rule entity should exist.
        let conn = storage.conn();
        let rule_count = crate::storage::crud::count_by_type(&conn, "rule").unwrap();
        assert_eq!(rule_count, 0);
    }

    #[test]
    fn promotes_observation_with_enough_supporters() {
        let storage = setup();
        let dir = tempfile::tempdir().unwrap();
        let cogz_dir = dir.path().join(".cogz");
        std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

        {
            let conn = storage.conn();
            insert_entity(
                &conn,
                &Entity::new(
                    "obs-1",
                    "observation",
                    "Test Obs",
                    "Always use batched queries",
                ),
            )
            .unwrap();
            for i in 2..=4 {
                insert_entity(
                    &conn,
                    &Entity::new(&format!("sup-{i}"), "observation", &format!("Sup {i}"), "c"),
                )
                .unwrap();
                add_supports_edge(&conn, &format!("sup-{i}"), "obs-1");
            }
        }

        let results = run_promotion(&storage, &cogz_dir, &config(3), false).unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].new_rule_id.is_empty());

        let conn = storage.conn();
        let rule_count = crate::storage::crud::count_by_type(&conn, "rule").unwrap();
        assert_eq!(rule_count, 1);

        // Verify derived_from edge.
        let edges = crate::storage::edges::get_edges_from(&conn, &results[0].new_rule_id).unwrap();
        assert!(
            edges
                .iter()
                .any(|e| e.edge_type == "derived_from" && e.target_id == "obs-1")
        );

        // Verify rule_promoted event.
        let events =
            crate::storage::events::get_events_for_entity(&conn, &results[0].new_rule_id).unwrap();
        assert!(events.iter().any(|e| e.event_type == "rule_promoted"));
    }

    #[test]
    fn no_promotion_below_threshold() {
        let storage = setup();
        let dir = tempfile::tempdir().unwrap();
        let cogz_dir = dir.path().join(".cogz");
        std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();

        {
            let conn = storage.conn();
            insert_entity(&conn, &Entity::new("obs-1", "observation", "Obs 1", "c")).unwrap();
            insert_entity(&conn, &Entity::new("sup-1", "observation", "Sup 1", "c")).unwrap();
            add_supports_edge(&conn, "sup-1", "obs-1");
        }

        let results = run_promotion(&storage, &cogz_dir, &config(3), false).unwrap();
        assert!(results.is_empty());
    }
}
