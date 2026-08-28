//! Reciprocal Rank Fusion (RRF).
//!
//! Pure function — no DB access, no I/O. Given ranked lists from
//! multiple sources (FTS5, vector search), produces a fused ranked
//! list. The RRF score for a document is:
//!
//! ```text
//! score(d) = Σ_i  weight_i / (k + rank_i(d))
//! ```
//!
//! where `rank` is 1-based and `k` is a tuning constant (default 60).

use std::collections::HashMap;

/// Fuse multiple ranked lists into a single ranked list.
///
/// Each input list is a vector of entity IDs in rank order (best first).
/// The weight controls how much each list contributes to the final score.
/// Returns `(entity_id, fused_score)` pairs sorted by descending score.
pub fn fuse(ranked_lists: &[(&[String], f64)], k: u32) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();

    for (ids, weight) in ranked_lists {
        for (rank, id) in ids.iter().enumerate() {
            let rank_1based = (rank + 1) as f64;
            let contribution = weight / (k as f64 + rank_1based);
            *scores.entry(id.clone()).or_default() += contribution;
        }
    }

    let mut results: Vec<(String, f64)> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuse_two_lists() {
        let fts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vec = vec!["b".to_string(), "a".to_string(), "d".to_string()];

        let fused = fuse(&[(&fts, 0.4), (&vec, 0.6)], 60);

        // 'a' is rank 1 in FTS, rank 2 in vec
        // 'b' is rank 2 in FTS, rank 1 in vec
        // b should rank higher because it's rank 1 in the higher-weighted vec list
        assert_eq!(fused[0].0, "b");
        assert_eq!(fused[1].0, "a");
        // 'c' only in FTS, 'd' only in vec
        assert!(fused.iter().any(|(id, _)| id == "c"));
        assert!(fused.iter().any(|(id, _)| id == "d"));
    }

    #[test]
    fn fuse_empty_lists() {
        let fused = fuse(&[], 60);
        assert!(fused.is_empty());
    }

    #[test]
    fn fuse_single_list() {
        let fts = vec!["a".to_string(), "b".to_string()];
        let fused = fuse(&[(&fts, 1.0)], 60);
        assert_eq!(fused[0].0, "a");
        assert_eq!(fused[1].0, "b");
        // score = 1/(60+1) for rank 1, 1/(60+2) for rank 2
        assert!((fused[0].1 - 1.0 / 61.0).abs() < 1e-10);
        assert!((fused[1].1 - 1.0 / 62.0).abs() < 1e-10);
    }

    #[test]
    fn fuse_one_empty_list() {
        let fts = vec!["a".to_string(), "b".to_string()];
        let vec: Vec<String> = vec![];

        let fused = fuse(&[(&fts, 0.4), (&vec, 0.6)], 60);
        assert_eq!(fused.len(), 2);
        // Only FTS contributed, so scores are scaled by fts_weight
        assert!((fused[0].1 - 0.4 / 61.0).abs() < 1e-10);
    }

    #[test]
    fn fuse_disjoint_lists() {
        let fts = vec!["a".to_string(), "b".to_string()];
        let vec = vec!["c".to_string(), "d".to_string()];

        let fused = fuse(&[(&fts, 0.5), (&vec, 0.5)], 60);
        assert_eq!(fused.len(), 4);
        // All have rank 1 or 2 with equal weight, so a and c tie, b and d tie
        assert!((fused[0].1 - 0.5 / 61.0).abs() < 1e-10);
    }

    #[test]
    fn fuse_weights_affect_ordering() {
        let fts = vec!["a".to_string(), "b".to_string()];
        let vec = vec!["b".to_string(), "a".to_string()];

        // With equal weights, a (rank 1 FTS) and b (rank 1 vec) tie
        let fused_equal = fuse(&[(&fts, 0.5), (&vec, 0.5)], 60);
        let score_a = fused_equal.iter().find(|(id, _)| id == "a").unwrap().1;
        let score_b = fused_equal.iter().find(|(id, _)| id == "b").unwrap().1;
        assert!((score_a - score_b).abs() < 1e-10);

        // With FTS weighted higher, a (rank 1 FTS) should win
        let fused_fts_heavy = fuse(&[(&fts, 0.9), (&vec, 0.1)], 60);
        assert_eq!(fused_fts_heavy[0].0, "a");
    }

    #[test]
    fn fuse_scores_are_descending() {
        let fts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vec = vec!["c".to_string(), "a".to_string()];

        let fused = fuse(&[(&fts, 0.4), (&vec, 0.6)], 60);
        for i in 1..fused.len() {
            assert!(fused[i - 1].1 >= fused[i].1);
        }
    }
}
