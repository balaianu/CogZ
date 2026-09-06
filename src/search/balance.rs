//! Source-type balance — detects how much weight to give code vs
//! knowledge entities using two complementary signals.
//!
//! **Signal 1: KNN distance spread (semantic match strength).**
//! sqlite-vec returns L2 distances. A strong match in an embedding
//! space produces a distance gradient — the nearest neighbor is much
//! closer than the k-th. A weak match produces uniform distances.
//! We normalize each batch to [0, 1] and convert to similarity.
//!
//! **Signal 2: FTS pool size ratio (lexical match breadth).**
//! With OR semantics, a query matches more entities in the collection
//! whose vocabulary aligns with the query terms. A code query like
//! "graph expansion BFS edges" matches more code entities (functions/
//! files with those terms) than knowledge entities. This ratio captures
//! query intent — which collection the query is *about* — that the
//! distance signal misses (documentation about X can be semantically
//! closer to "X" than the implementation of X).
//!
//! The combined proportion is a weighted average:
//! `proportion = α × knn_signal + (1-α) × fts_pool_signal`
//! with α = 0.5 by default (equal weight to both signals).
//!
//! A floor (`min_proportion`) ensures neither source type is completely
//! suppressed.

/// Weight given to the KNN distance signal vs the FTS pool signal.
/// 0.5 = equal weight. Higher = trust semantic similarity more.
const KNN_SIGNAL_WEIGHT: f64 = 0.5;

/// Detect the proportion of weight to give code vs knowledge entities
/// in RRF fusion, combining KNN distance spread and FTS pool size.
///
/// Takes the top-k distances from each embedding space's KNN search
/// and the count of FTS matches in each pool. Returns
/// `(code_proportion, knowledge_proportion)` summing to 1.0.
///
/// When neither KNN nor FTS provides signal, returns (0.5, 0.5).
pub fn detect_proportions(
    code_distances: &[f32],
    knowledge_distances: &[f32],
    code_fts_count: usize,
    knowledge_fts_count: usize,
    code_collection_size: usize,
    knowledge_collection_size: usize,
    min_proportion: f64,
) -> (f64, f64) {
    let knn_code = mean_top_k_similarity(code_distances, 5);
    let knn_knowledge = mean_top_k_similarity(knowledge_distances, 5);

    let fts_code = fts_pool_signal(
        code_fts_count,
        knowledge_fts_count,
        code_collection_size,
        knowledge_collection_size,
    );
    let fts_knowledge = 1.0 - fts_code;

    // Combine signals: weighted average of KNN and FTS pool signals.
    // If one signal is unavailable (e.g. no KNN results), fall back
    // to the other signal entirely.
    let (code_score, knowledge_score) = if knn_code <= 0.0 && knn_knowledge <= 0.0 {
        // No KNN signal — use FTS pool ratio only
        (fts_code, fts_knowledge)
    } else if code_fts_count == 0 && knowledge_fts_count == 0 {
        // No FTS signal — use KNN only
        (knn_code, knn_knowledge)
    } else {
        // Both signals available — weighted average
        let combined_code = KNN_SIGNAL_WEIGHT * knn_code + (1.0 - KNN_SIGNAL_WEIGHT) * fts_code;
        let combined_knowledge =
            KNN_SIGNAL_WEIGHT * knn_knowledge + (1.0 - KNN_SIGNAL_WEIGHT) * fts_knowledge;
        (combined_code, combined_knowledge)
    };

    if code_score <= 0.0 && knowledge_score <= 0.0 {
        return (0.5, 0.5);
    }

    if code_score <= 0.0 {
        return (min_proportion, 1.0 - min_proportion);
    }

    if knowledge_score <= 0.0 {
        return (1.0 - min_proportion, min_proportion);
    }

    let total = code_score + knowledge_score;
    let mut code_prop = code_score / total;
    let mut knowledge_prop = knowledge_score / total;

    // Apply floor — ensure neither source type is completely suppressed
    let floor = min_proportion;
    if code_prop < floor {
        code_prop = floor;
        knowledge_prop = 1.0 - floor;
    } else if knowledge_prop < floor {
        knowledge_prop = floor;
        code_prop = 1.0 - floor;
    }

    (code_prop, knowledge_prop)
}

/// Convert FTS match counts to a [0, 1] proportion for code, using
/// sqrt-dampened counts. Code entities outnumber knowledge entities
/// (e.g. 1270 vs 57), so raw counts bias toward code. But match-rate
/// normalization (count/collection_size) overcorrects — with 57
/// knowledge entities, any match produces a huge rate.
///
/// sqrt dampening is the middle ground: it reduces the large-collection
/// advantage without making small collections dominate. For example,
/// 20 code matches vs 5 knowledge matches: raw ratio = 0.80, sqrt
/// ratio = sqrt(20)/(sqrt(20)+sqrt(5)) = 4.47/6.71 = 0.67. The code
/// advantage is preserved but softened.
fn fts_pool_signal(
    code_count: usize,
    knowledge_count: usize,
    _code_collection_size: usize,
    _knowledge_collection_size: usize,
) -> f64 {
    let total = code_count + knowledge_count;
    if total == 0 {
        return 0.5;
    }

    let code_sqrt = (code_count as f64).sqrt();
    let knowledge_sqrt = (knowledge_count as f64).sqrt();
    let sqrt_total = code_sqrt + knowledge_sqrt;
    if sqrt_total > 0.0 {
        code_sqrt / sqrt_total
    } else {
        0.5
    }
}

/// Convert vec0 L2 distances to relative similarities and average the
/// top-k. Embeddings are L2-normalized at inference time, so distances
/// range [0, 2] (0 = identical, 2 = opposite). We normalize each batch
/// to [0, 1] by dividing by the max distance, then convert to
/// similarity: `1.0 - d/d_max`. The nearest neighbor gets similarity
/// ~1.0, the farthest gets ~0.0. This is a relative measure of how
/// close the query is to the nearest entities in this space compared
/// to the farthest in the same batch — it detects distance *gradient*
/// (strong vs weak match), not absolute similarity.
fn mean_top_k_similarity(distances: &[f32], k: usize) -> f64 {
    if distances.is_empty() {
        return 0.0;
    }
    let k = k.min(distances.len());
    let top_k = &distances[..k];
    let max_dist = top_k.iter().cloned().fold(0.0f32, f32::max);
    if max_dist <= 0.0 {
        // All distances are 0 (exact matches) — max similarity
        return 1.0;
    }
    let sum: f64 = top_k
        .iter()
        .map(|&d| 1.0 - (d as f64 / max_dist as f64))
        .sum();
    sum / k as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── KNN-only signal (FTS counts = 0) ──────────────────────────

    #[test]
    fn empty_distances_no_fts_return_equal_split() {
        let (c, k) = detect_proportions(&[], &[], 0, 0, 0, 0, 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }

    #[test]
    fn knn_code_favored_when_closer() {
        let code = vec![17.0, 18.0, 19.0, 20.0, 21.0];
        let knowledge = vec![12.0, 12.1, 12.2, 12.3, 12.4];
        let (c, k) = detect_proportions(&code, &knowledge, 0, 0, 0, 0, 0.2);
        assert!(c > 0.5, "code proportion should be > 0.5, got {c}");
        assert!(k < 0.5, "knowledge proportion should be < 0.5, got {k}");
        assert!((c + k - 1.0).abs() < 1e-10);
    }

    #[test]
    fn knn_knowledge_favored_when_closer() {
        let code = vec![18.0, 18.1, 18.2, 18.3, 18.4];
        let knowledge = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let (c, k) = detect_proportions(&code, &knowledge, 0, 0, 0, 0, 0.2);
        assert!(k > 0.5, "knowledge proportion should be > 0.5, got {k}");
        assert!(c < 0.5, "code proportion should be < 0.5, got {c}");
    }

    #[test]
    fn knn_floor_prevents_suppression() {
        let code = vec![10.0, 15.0, 20.0, 25.0, 30.0];
        let knowledge = vec![12.0, 12.01, 12.02, 12.03, 12.04];
        let (c, k) = detect_proportions(&code, &knowledge, 0, 0, 0, 0, 0.2);
        assert!(
            k >= 0.2 - 1e-10,
            "knowledge should be at least floor, got {k}"
        );
        assert!(c <= 0.8 + 1e-10, "code should be at most 1-floor, got {c}");
    }

    #[test]
    fn knn_only_code_returns_code_heavy_with_floor() {
        let code = vec![10.0, 12.0, 14.0, 16.0, 18.0];
        let (c, k) = detect_proportions(&code, &[], 0, 0, 0, 0, 0.2);
        assert!((c - 0.8).abs() < 1e-10);
        assert!((k - 0.2).abs() < 1e-10);
    }

    #[test]
    fn knn_only_knowledge_returns_knowledge_heavy_with_floor() {
        let knowledge = vec![10.0, 12.0, 14.0, 16.0, 18.0];
        let (c, k) = detect_proportions(&[], &knowledge, 0, 0, 0, 0, 0.2);
        assert!((c - 0.2).abs() < 1e-10);
        assert!((k - 0.8).abs() < 1e-10);
    }

    #[test]
    fn knn_equal_spreads_return_equal_split() {
        let code = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let knowledge = vec![20.0, 22.0, 24.0, 26.0, 28.0];
        let (c, k) = detect_proportions(&code, &knowledge, 0, 0, 0, 0, 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }

    // ─── FTS-only signal (no KNN distances) ────────────────────────

    #[test]
    fn fts_only_code_heavy_pool() {
        // 8 code FTS matches, 2 knowledge → code should be favored
        let (c, k) = detect_proportions(&[], &[], 8, 2, 100, 50, 0.2);
        assert!(c > 0.5, "code should be favored by FTS pool, got {c}");
        assert!(k < 0.5, "knowledge should be < 0.5, got {k}");
    }

    #[test]
    fn fts_only_knowledge_heavy_pool() {
        let (c, k) = detect_proportions(&[], &[], 2, 8, 100, 50, 0.2);
        assert!(k > 0.5, "knowledge should be favored by FTS pool, got {k}");
        assert!(c < 0.5, "code should be < 0.5, got {c}");
    }

    #[test]
    fn fts_equal_pools_return_equal_split() {
        // Equal match rates: 5/100 = 5/100 → equal split
        let (c, k) = detect_proportions(&[], &[], 5, 5, 100, 100, 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }

    // ─── Combined signal ───────────────────────────────────────────

    #[test]
    fn combined_signals_agree_on_code() {
        // Both KNN and FTS say code → strong code favor
        let code = vec![10.0, 15.0, 20.0, 25.0, 30.0]; // wide spread
        let knowledge = vec![12.0, 12.1, 12.2, 12.3, 12.4]; // narrow
        let (c, k) = detect_proportions(&code, &knowledge, 8, 2, 100, 50, 0.2);
        assert!(c > 0.6, "code should be strongly favored, got {c}");
        assert!(k < 0.4, "knowledge should be < 0.4, got {k}");
    }

    #[test]
    fn combined_signals_agree_on_knowledge() {
        let code = vec![18.0, 18.1, 18.2, 18.3, 18.4]; // narrow
        let knowledge = vec![10.0, 15.0, 20.0, 25.0, 30.0]; // wide spread
        let (c, k) = detect_proportions(&code, &knowledge, 2, 8, 100, 50, 0.2);
        assert!(k > 0.6, "knowledge should be strongly favored, got {k}");
        assert!(c < 0.4, "code should be < 0.4, got {c}");
    }

    #[test]
    fn combined_signals_disagree_balance_toward_fts_intent() {
        // KNN says knowledge (wider spread), FTS says code (more matches).
        // This is the "documentation about X vs implementation of X" case.
        // The combined signal should be closer to 50/50 than either alone.
        let code = vec![18.0, 18.1, 18.2, 18.3, 18.4]; // narrow (KNN: knowledge)
        let knowledge = vec![10.0, 15.0, 20.0, 25.0, 30.0]; // wide (KNN: knowledge)
        let (c, k) = detect_proportions(&code, &knowledge, 8, 2, 100, 50, 0.2);
        // FTS pulls code up, KNN pulls knowledge up. Result should be
        // more balanced than pure KNN (which would give ~0.2 code).
        assert!(
            c > 0.3,
            "FTS signal should pull code above KNN-only level, got {c}"
        );
        assert!((c + k - 1.0).abs() < 1e-10);
    }

    #[test]
    fn combined_equal_signals_return_equal_split() {
        // Equal KNN spreads + equal FTS match rates → equal split
        let code = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let knowledge = vec![20.0, 22.0, 24.0, 26.0, 28.0];
        let (c, k) = detect_proportions(&code, &knowledge, 5, 5, 100, 100, 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }

    // ─── Edge cases ────────────────────────────────────────────────

    #[test]
    fn uses_top_5_only() {
        let code = vec![10.0, 10.5, 11.0, 25.0, 25.5, 26.0];
        let knowledge = vec![12.0, 12.1, 12.2];
        let (c, k) = detect_proportions(&code, &knowledge, 0, 0, 0, 0, 0.2);
        assert!(c > 0.5, "code should be favored, got {c}");
        assert!((c + k - 1.0).abs() < 1e-10);
    }

    #[test]
    fn proportions_sum_to_one() {
        let code = vec![15.0, 16.0, 17.0, 18.0, 19.0];
        let knowledge = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let (c, k) = detect_proportions(&code, &knowledge, 3, 7, 100, 50, 0.15);
        assert!((c + k - 1.0).abs() < 1e-10);
    }

    // ─── Sqrt-dampened FTS signal ──────────────────────────────────

    #[test]
    fn fts_sqrt_dampens_code_advantage() {
        // 20 code matches vs 5 knowledge: raw ratio = 0.80,
        // sqrt ratio = sqrt(20)/(sqrt(20)+sqrt(5)) ≈ 0.67.
        // Code still favored but less aggressively than raw counts.
        let (c, _k) = detect_proportions(&[], &[], 20, 5, 1270, 57, 0.2);
        assert!(c > 0.5, "code should still be favored, got c={c}");
        assert!(c < 0.8, "but less aggressively than raw, got c={c}");
    }

    #[test]
    fn fts_sqrt_equal_counts_equal_split() {
        let (c, k) = detect_proportions(&[], &[], 5, 5, 1000, 50, 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }
}
