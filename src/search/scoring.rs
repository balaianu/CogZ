//! Composite scoring — combines multiple signals for ranking.
//!
//! Used by cold start (no query similarity) and optionally by search
//! to boost frequently-accessed and high-confidence entities.
//!
//! score = w_semantic * similarity
//!       + w_importance * importance
//!       + w_recency * recency_decay
//!       + w_frequency * log(access_count + 1)
//!
//! For cold start (no query):
//! score = w_importance * confidence
//!       + w_frequency * log(access_count + 1)
//!       + w_recency * recency_decay

use crate::storage::crud::Entity;

/// Weights for composite scoring. Defaults follow research-recommended
/// ratios: semantic 40%, importance 30%, recency 20%, frequency 10%.
#[derive(Debug, Clone)]
pub struct ScoreWeights {
    pub semantic: f64,
    pub importance: f64,
    pub recency: f64,
    pub frequency: f64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            semantic: 0.4,
            importance: 0.3,
            recency: 0.2,
            frequency: 0.1,
        }
    }
}

/// Compute a composite score for an entity in cold-start mode (no query
/// similarity). Uses confidence from properties, access frequency, and
/// recency decay.
pub fn cold_start_score(
    entity: &Entity,
    access_count: i64,
    now: &chrono::DateTime<chrono::Utc>,
    weights: &ScoreWeights,
) -> f64 {
    let importance = entity_confidence(entity);
    let recency = recency_decay(&entity.created_at, now);
    let frequency = (access_count as f64 + 1.0).ln();

    weights.importance * importance + weights.frequency * frequency + weights.recency * recency
}

/// Exponential recency decay: returns 1.0 for entities created now,
/// decaying to ~0.1 over ~30 days, ~0.01 over ~90 days.
/// Uses a half-life of 14 days.
pub fn recency_decay(created_at: &str, now: &chrono::DateTime<chrono::Utc>) -> f64 {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return 0.0;
    };
    let age = now.signed_duration_since(ts.with_timezone(&chrono::Utc));
    let days = age.num_seconds() as f64 / 86400.0;
    // Half-life of 14 days: 2^(-days/14)
    2.0_f64.powf(-days / 14.0)
}

/// Extract confidence from entity properties. Defaults to 0.5 if not
/// present. Rules and knowledge may have a `confidence` field in
/// frontmatter that gets stored in properties.
fn entity_confidence(entity: &Entity) -> f64 {
    if let Some(conf) = entity.properties.get("confidence")
        && let Some(n) = conf.as_f64()
    {
        return n;
    }
    // Rules default to high confidence (they're curated)
    if entity.r#type == "rule" { 0.8 } else { 0.5 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_decay_is_one_for_now() {
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let decay = recency_decay(&ts, &now);
        assert!((decay - 1.0).abs() < 0.01, "decay should be ~1.0 for now");
    }

    #[test]
    fn recency_decay_decreases_with_age() {
        let now = chrono::Utc::now();
        let old = (now - chrono::Duration::days(30)).to_rfc3339();
        let recent = (now - chrono::Duration::days(1)).to_rfc3339();

        let old_decay = recency_decay(&old, &now);
        let recent_decay = recency_decay(&recent, &now);

        assert!(recent_decay > old_decay, "recent should decay slower");
        assert!(old_decay < 0.3, "30-day-old should be < 0.3");
    }

    #[test]
    fn cold_start_score_prefers_rules_over_observations() {
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let weights = ScoreWeights::default();

        let rule = Entity::new("r1", "rule", "T", "c");
        let obs = Entity::new("o1", "observation", "T", "c");

        let rule_score = cold_start_score(&rule, 0, &now, &weights);
        let obs_score = cold_start_score(&obs, 0, &now, &weights);

        assert!(
            rule_score > obs_score,
            "rules should score higher than observations by default"
        );
    }

    #[test]
    fn cold_start_score_boosts_frequently_accessed() {
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let weights = ScoreWeights::default();

        let entity = Entity::new("e1", "knowledge", "T", "c");
        let low_access = cold_start_score(&entity, 0, &now, &weights);
        let high_access = cold_start_score(&entity, 50, &now, &weights);

        assert!(
            high_access > low_access,
            "frequently accessed should score higher"
        );
    }
}
