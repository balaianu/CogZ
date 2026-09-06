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
use crate::files::{EntityFile, FileEntityType};
use crate::storage::StorageError;
use crate::storage::crud::Entity;
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
    use crate::storage::graph::get_edges_involving_batch;

    let observations = {
        let conn = storage.conn();
        get_entities_by_type(&conn, "observation", Some("active"), 500)?
    };

    if observations.is_empty() {
        return Ok(Vec::new());
    }

    // Batch-fetch all edges involving these observations in a single
    // query set, instead of one get_edges_to call per observation.
    let obs_ids: Vec<String> = observations.iter().map(|o| o.id.clone()).collect();
    let all_edges = {
        let conn = storage.conn();
        get_edges_involving_batch(&conn, &obs_ids)?
    };

    // Group edges by target_id (observations receive edges as targets).
    use std::collections::HashMap;
    let mut edges_by_target: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for (source_id, target_id, edge_type) in &all_edges {
        edges_by_target.entry(target_id.clone()).or_default().push((
            source_id.clone(),
            target_id.clone(),
            edge_type.clone(),
        ));
    }

    let mut candidates = Vec::new();
    for obs in &observations {
        let edges = edges_by_target
            .get(&obs.id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // Skip if already promoted — a derived_from edge from a rule
        // means this observation was already promoted.
        let already_promoted = edges
            .iter()
            .any(|(source_id, _, edge_type)| edge_type == "derived_from" && source_id != &obs.id);
        if already_promoted {
            continue;
        }

        let supporting_ids: Vec<String> = edges
            .iter()
            .filter(|(_, _, edge_type)| edge_type == "supports")
            .map(|(source_id, _, _)| source_id.clone())
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

    // Scan for secrets before writing. Rules are committed to git.
    if let Some(scan) = crate::security::scan_content(&rule_file.title, &rule_file.body) {
        tracing::warn!(
            "skipping promotion of observation {}: content contains a suspected {}",
            candidate.observation.id,
            scan.kind
        );
        return Err(StorageError::File(format!(
            "observation {} contains a suspected {} — not promoting to a committed rule file",
            candidate.observation.id, scan.kind
        )));
    }

    // Write the file first (file-first invariant). Use file_path_safe
    // to avoid collisions with existing rules. Use atomic write to
    // prevent concurrent consolidations from overwriting each other.
    let file_path = rule_file.file_path_safe(cogz_dir);
    let file_path = crate::mcp::helpers::write_entity_file_atomic(&file_path, &rule_file, cogz_dir)
        .map_err(|e| StorageError::File(format!("{}: {}", file_path.display(), e)))?;

    let rule_id = rule_file.id.clone();

    // Sync the file to the DB via sync_single_file. This ensures the
    // DB entity (properties, content_hash, edges) exactly matches the
    // canonical file, and survives `cogz reset` + `cogz index`.
    // sync_single_file calls sync_references internally, which creates
    // the derived_from and supports edges from the frontmatter.
    let relative_path = file_path
        .strip_prefix(cogz_dir)
        .unwrap_or(&file_path)
        .to_string_lossy()
        .to_string();
    let sync_result = crate::files::sync_single_file(storage, cogz_dir, &relative_path);
    if let Some(err) = sync_result.errors.first() {
        return Err(StorageError::File(format!(
            "sync failed for promoted rule {}: {}",
            file_path.display(),
            err.error
        )));
    }

    // Record the promotion event.
    let conn = storage.conn();
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
