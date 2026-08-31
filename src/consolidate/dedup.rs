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
use crate::storage::crud::Entity;
use crate::storage::embeddings::knn_search;
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
    let neighbors = knn_search(conn, new_embedding, 5).ok()?;

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
mod tests {
    use super::*;
    use crate::storage::crud::{Entity, insert_entity};
    use crate::storage::ensure_vec_extension;
    use crate::storage::schema::run_migrations;

    fn setup() -> Connection {
        ensure_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, 768).unwrap();
        conn
    }

    fn config() -> ConsolidationConfig {
        ConsolidationConfig {
            dedup_threshold: 0.92,
            title_match_threshold: 0.85,
            contradiction_check: true,
            promotion_threshold: 3,
        }
    }

    #[test]
    fn exact_title_match_detected() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "FTS5 ranking bug", "content"),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new("u2", "observation", "FTS5 ranking bug", "content"),
        )
        .unwrap();

        let result = check_duplicate(
            &conn,
            "u2",
            "FTS5 ranking bug",
            "observation",
            None,
            &config(),
        );
        let warning = result.duplicate_warning.expect("should detect duplicate");
        assert_eq!(warning.existing_id, "u1");
        assert_eq!(warning.title_match, "exact");
        assert_eq!(warning.similarity, 1.0);
    }

    #[test]
    fn case_insensitive_title_match() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "knowledge", "Search Architecture", "c"),
        )
        .unwrap();
        insert_entity(
            &conn,
            &Entity::new("u2", "knowledge", "search architecture", "c"),
        )
        .unwrap();

        let result = check_duplicate(
            &conn,
            "u2",
            "search architecture",
            "knowledge",
            None,
            &config(),
        );
        assert!(result.duplicate_warning.is_some());
        assert_eq!(result.duplicate_warning.unwrap().title_match, "exact");
    }

    #[test]
    fn fuzzy_title_match_detected() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "rule", "Use parameterized queries", "c"),
        )
        .unwrap();

        // Different punctuation/case, same normalized form
        let result = check_duplicate(
            &conn,
            "u2",
            "use-parameterized queries!",
            "rule",
            None,
            &config(),
        );
        assert!(result.duplicate_warning.is_some());
        assert_eq!(result.duplicate_warning.unwrap().title_match, "fuzzy");
    }

    #[test]
    fn no_duplicate_when_titles_differ() {
        let conn = setup();
        insert_entity(
            &conn,
            &Entity::new("u1", "observation", "Bug in search", "c"),
        )
        .unwrap();

        let result = check_duplicate(
            &conn,
            "u2",
            "Feature request: dark mode",
            "observation",
            None,
            &config(),
        );
        assert!(result.duplicate_warning.is_none());
        assert!(!result.dedup_flagged);
    }

    #[test]
    fn skips_self_in_title_match() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "Same title", "c")).unwrap();

        let result = check_duplicate(&conn, "u1", "Same title", "observation", None, &config());
        assert!(result.duplicate_warning.is_none());
    }

    #[test]
    fn embedding_similarity_flagged() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();
        insert_entity(&conn, &Entity::new("u2", "observation", "B", "c")).unwrap();

        // Identical embeddings → distance ~0 → similarity ~1.0
        let embedding = vec![0.1_f32; 768];
        crate::storage::embeddings::insert_embedding(&conn, "u1", &embedding).unwrap();

        let result = check_duplicate(&conn, "u2", "B", "observation", Some(&embedding), &config());
        assert!(result.dedup_flagged);
        assert!(result.duplicate_warning.is_some());
    }

    #[test]
    fn no_embedding_check_without_embedding() {
        let conn = setup();
        insert_entity(&conn, &Entity::new("u1", "observation", "A", "c")).unwrap();

        let result = check_duplicate(
            &conn,
            "u2",
            "Different title",
            "observation",
            None,
            &config(),
        );
        assert!(!result.dedup_flagged);
        assert!(result.duplicate_warning.is_none());
    }

    #[test]
    fn normalize_title_strips_punctuation() {
        assert_eq!(
            normalize_title("Use-Parameterized Queries!"),
            "use parameterized queries"
        );
        assert_eq!(normalize_title("FTS5: Ranking Bug"), "fts5 ranking bug");
        assert_eq!(normalize_title("  multiple   spaces  "), "multiple spaces");
    }
}
