//! Duplicate detection — title match and embedding similarity.
//!
//! Two checks run on every insert:
//! 1. Title match — exact (case-insensitive) or fuzzy (normalized
//!    comparison). Returns a `DuplicateWarning` with a suggestion.
//! 2. Embedding similarity — cosine similarity via KNN search, if
//!    the new entity has an embedding and the model is available.
//!    Sets `dedup_flagged` if similarity exceeds the configured
//!    threshold.
//!
//! Both checks degrade gracefully: without embeddings, only title
//! match runs. Without any existing entities, no checks run.

use rusqlite::Connection;

use crate::config::ConsolidationConfig;
use crate::embed::NliModel;
use crate::storage::crud::Entity;
use crate::storage::embeddings::{EmbeddingSpace, knn_search};
use crate::storage::query::get_entities_by_type;

/// Result of a dedup check on a newly inserted entity.
#[derive(Debug, Clone)]
pub struct DedupResult {
    /// True if embedding similarity exceeded the dedup threshold.
    pub dedup_flagged: bool,
    /// True if an NLI contradiction was detected. Set by
    /// `contradict::check_contradiction`, not by dedup itself.
    pub contradiction_flagged: bool,
    /// Warning with details if a title match or high similarity was
    /// found. `None` if no duplicate detected.
    pub duplicate_warning: Option<DuplicateWarning>,
}

impl DedupResult {
    /// Empty result — no flags, no warning. Used when dedup is skipped
    /// or when no duplicates are found.
    pub fn empty() -> Self {
        Self {
            dedup_flagged: false,
            contradiction_flagged: false,
            duplicate_warning: None,
        }
    }
}

/// A warning that a newly created entity may duplicate an existing one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DuplicateWarning {
    pub existing_id: String,
    pub existing_title: String,
    pub similarity: f64,
    /// "exact", "fuzzy", or "embedding"
    pub title_match: String,
    pub suggestion: String,
}

/// Run dedup checks against existing entities of the same type.
///
/// `new_id` is the UUID of the just-inserted entity. `new_title` is
/// its title. `new_embedding` is its embedding vector, if available.
/// `entity_type` is the string form ("observation", "rule", "knowledge").
pub fn check_duplicate(
    conn: &Connection,
    new_id: &str,
    new_title: &str,
    entity_type: &str,
    new_embedding: Option<&[f32]>,
    config: &ConsolidationConfig,
) -> DedupResult {
    let existing = match get_entities_by_type(conn, entity_type, None, 100) {
        Ok(e) => e,
        Err(_) => return DedupResult::empty(),
    };

    // 1. Title match check
    if let Some(warning) =
        check_title_match(&existing, new_id, new_title, config.title_match_threshold)
    {
        return DedupResult {
            dedup_flagged: false,
            contradiction_flagged: false,
            duplicate_warning: Some(warning),
        };
    }

    // 2. Embedding similarity check
    if let Some(embedding) = new_embedding
        && let Some((dedup_flagged, warning)) =
            check_embedding_similarity(conn, embedding, &existing, new_id, config.dedup_threshold)
    {
        return DedupResult {
            dedup_flagged,
            contradiction_flagged: false,
            duplicate_warning: warning,
        };
    }

    DedupResult::empty()
}

/// Check for title matches among existing entities.
fn check_title_match(
    existing: &[Entity],
    new_id: &str,
    new_title: &str,
    _threshold: f64,
) -> Option<DuplicateWarning> {
    let new_lower = new_title.to_lowercase();
    let new_normalized = normalize_title(new_title);

    for entity in existing {
        if entity.id == new_id {
            continue;
        }
        let Some(ref title) = entity.title else {
            continue;
        };

        // Exact match (case-insensitive)
        if title.to_lowercase() == new_lower {
            return Some(DuplicateWarning {
                existing_id: entity.id.clone(),
                existing_title: title.clone(),
                similarity: 1.0,
                title_match: "exact".to_string(),
                suggestion: format!(
                    "This entry may duplicate existing entry {}. Use update_knowledge(id=\"{}\") to update the existing entry, or ignore if this is a distinct entry.",
                    entity.id, entity.id
                ),
            });
        }

        // Fuzzy match (normalized comparison)
        let existing_normalized = normalize_title(title);
        if new_normalized == existing_normalized && !new_normalized.is_empty() {
            return Some(DuplicateWarning {
                existing_id: entity.id.clone(),
                existing_title: title.clone(),
                similarity: 0.95,
                title_match: "fuzzy".to_string(),
                suggestion: format!(
                    "This entry may duplicate existing entry {}. If correcting it, create a contradicts edge instead. If distinct, ignore this warning.",
                    entity.id
                ),
            });
        }
    }

    None
}

/// Check embedding similarity via KNN search.
fn check_embedding_similarity(
    conn: &Connection,
    new_embedding: &[f32],
    existing: &[Entity],
    new_id: &str,
    dedup_threshold: f64,
) -> Option<(bool, Option<DuplicateWarning>)> {
    let neighbors = knn_search(conn, EmbeddingSpace::Knowledge, new_embedding, 5).ok()?;

    let mut dedup_flagged = false;
    let mut best_warning: Option<DuplicateWarning> = None;

    for (entity_id, distance) in &neighbors {
        if entity_id == new_id {
            continue;
        }

        // sqlite-vec returns L2 distance. Convert to similarity:
        // similarity = 1 / (1 + distance)
        let similarity = 1.0 / (1.0 + *distance as f64);

        if similarity >= dedup_threshold {
            dedup_flagged = true;
            if best_warning.is_none()
                && let Some(entity) = existing.iter().find(|e| e.id == *entity_id)
            {
                best_warning = Some(DuplicateWarning {
                    existing_id: entity.id.clone(),
                    existing_title: entity.title.clone().unwrap_or_default(),
                    similarity,
                    title_match: "embedding".to_string(),
                    suggestion: format!(
                        "This entry is semantically very similar to existing entry {} (similarity {:.2}). Consider merging or ignoring if distinct.",
                        entity.id, similarity
                    ),
                });
            }
        }
    }

    if dedup_flagged || best_warning.is_some() {
        Some((dedup_flagged, best_warning))
    } else {
        None
    }
}

/// Confirm a potential duplicate pair using NLI bidirectional entailment.
/// True duplicates entail mutually: both A entails B and B entails A
/// must score above the threshold. A subset-fact (A entails B but not
/// vice versa) is not a duplicate.
///
/// Returns true if the pair is confirmed as duplicates, false otherwise.
/// Returns false if the NLI model is unavailable.
pub fn confirm_duplicate_nli(
    model: Option<&dyn NliModel>,
    text_a: &str,
    text_b: &str,
    threshold: f64,
) -> bool {
    let Some(model) = model else {
        return false;
    };

    let forward = model.classify(text_a, text_b);
    let reverse = model.classify(text_b, text_a);

    let min_entailment = match (forward, reverse) {
        (Ok(f), Ok(r)) => f.entailment.min(r.entailment),
        (Ok(f), Err(_)) => f.entailment,
        (Err(_), Ok(r)) => r.entailment,
        (Err(_), _) => return false,
    };

    min_entailment >= threshold as f32
}

/// Normalize a title for fuzzy comparison: lowercase, remove
/// punctuation, collapse whitespace.
fn normalize_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "dedup_tests.rs"]
mod tests;
