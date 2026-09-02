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

        // Skip if already promoted — a derived_from edge from a rule
        // means this observation was already promoted.
        let already_promoted = edges
            .iter()
            .any(|e| e.edge_type == "derived_from" && e.source_id != obs.id);
        if already_promoted {
            continue;
        }

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
#[path = "promote_tests.rs"]
mod tests;
