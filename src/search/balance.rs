//! Source-type balance — detects how much weight to give code vs
//! knowledge entities based on query embedding similarity scores.
//!
//! The problem: BM25 FTS scores aren't comparable across collections.
//! Knowledge entries are text-dense, so FTS ranks them higher even
//! when the query is about code. By comparing top-k vector similarity
//! scores from each embedding space, we detect which collection the
//! query belongs to and scale RRF weights accordingly.
//!
//! This is a simplified form of ReDDE (Si & Callan 2003): use the
//! similarity distribution from each collection to estimate how
//! relevant that collection is to the query.

/// Detect the proportion of weight to give code vs knowledge entities
/// in RRF fusion.
///
/// Takes the top-k distances from each embedding space's KNN search.
/// Distances are cosine distances from vec0 (0 = identical, 1 =
/// orthogonal, 2 = opposite). Converts to similarities, averages the
/// top-5, and derives proportions. A floor (`min_proportion`) ensures
/// neither source type is completely suppressed.
///
/// Returns `(code_proportion, knowledge_proportion)` summing to 1.0.
/// When neither space has results, returns (0.5, 0.5).
pub fn detect_proportions(
    code_distances: &[f32],
    knowledge_distances: &[f32],
    min_proportion: f64,
) -> (f64, f64) {
    let code_score = mean_top_k_similarity(code_distances, 5);
    let knowledge_score = mean_top_k_similarity(knowledge_distances, 5);

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

/// Convert vec0 L2 distances to relative similarities and average the
/// top-k. sqlite-vec returns L2 (Euclidean) distances, which for
/// high-dimensional vectors are typically 10-25 — not bounded to [0, 2]
/// like cosine distances. To make distances comparable across different
/// embedding spaces (code vs knowledge use different models), we
/// normalize each batch to [0, 1] by dividing by the max distance, then
/// convert to similarity: `1.0 - d/d_max`. The nearest neighbor gets
/// similarity ~1.0, the farthest gets ~0.0. This is a relative measure
/// of how close the query is to the nearest entities in this space
/// compared to the farthest in the same batch.
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

    #[test]
    fn empty_distances_return_equal_split() {
        let (c, k) = detect_proportions(&[], &[], 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }

    #[test]
    fn code_favored_when_closer() {
        // L2 distances: code has a wider spread (stronger match gradient),
        // knowledge is uniform (weak match). sqlite-vec returns L2
        // distances typically in the 10-25 range for 768d vectors.
        let code = vec![17.0, 18.0, 19.0, 20.0, 21.0];
        let knowledge = vec![12.0, 12.1, 12.2, 12.3, 12.4];
        let (c, k) = detect_proportions(&code, &knowledge, 0.2);
        assert!(c > 0.5, "code proportion should be > 0.5, got {c}");
        assert!(k < 0.5, "knowledge proportion should be < 0.5, got {k}");
        assert!((c + k - 1.0).abs() < 1e-10);
    }

    #[test]
    fn knowledge_favored_when_closer() {
        // Knowledge has a wider spread (stronger match), code is uniform
        let code = vec![18.0, 18.1, 18.2, 18.3, 18.4];
        let knowledge = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let (c, k) = detect_proportions(&code, &knowledge, 0.2);
        assert!(k > 0.5, "knowledge proportion should be > 0.5, got {k}");
        assert!(c < 0.5, "code proportion should be < 0.5, got {c}");
    }

    #[test]
    fn floor_prevents_suppression() {
        // Code has a very strong gradient, knowledge is nearly uniform
        let code = vec![10.0, 15.0, 20.0, 25.0, 30.0];
        let knowledge = vec![12.0, 12.01, 12.02, 12.03, 12.04];
        let (c, k) = detect_proportions(&code, &knowledge, 0.2);
        assert!(
            k >= 0.2 - 1e-10,
            "knowledge should be at least floor, got {k}"
        );
        assert!(c <= 0.8 + 1e-10, "code should be at most 1-floor, got {c}");
    }

    #[test]
    fn only_code_returns_code_heavy_with_floor() {
        let code = vec![10.0, 12.0, 14.0, 16.0, 18.0];
        let (c, k) = detect_proportions(&code, &[], 0.2);
        assert!((c - 0.8).abs() < 1e-10);
        assert!((k - 0.2).abs() < 1e-10);
    }

    #[test]
    fn only_knowledge_returns_knowledge_heavy_with_floor() {
        let knowledge = vec![10.0, 12.0, 14.0, 16.0, 18.0];
        let (c, k) = detect_proportions(&[], &knowledge, 0.2);
        assert!((c - 0.2).abs() < 1e-10);
        assert!((k - 0.8).abs() < 1e-10);
    }

    #[test]
    fn equal_spreads_return_equal_split() {
        // Same relative spread in both spaces → equal proportions
        let code = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let knowledge = vec![20.0, 22.0, 24.0, 26.0, 28.0];
        let (c, k) = detect_proportions(&code, &knowledge, 0.2);
        assert!((c - 0.5).abs() < 1e-10);
        assert!((k - 0.5).abs() < 1e-10);
    }

    #[test]
    fn uses_top_5_only() {
        // 3 close + 3 far in code (wide spread), only 3 in knowledge (narrow)
        let code = vec![10.0, 10.5, 11.0, 25.0, 25.5, 26.0];
        let knowledge = vec![12.0, 12.1, 12.2];
        let (c, k) = detect_proportions(&code, &knowledge, 0.2);
        // Code top-5 has wider spread → higher mean similarity → code favored
        assert!(c > 0.5, "code should be favored, got {c}");
        assert!((c + k - 1.0).abs() < 1e-10);
    }

    #[test]
    fn proportions_sum_to_one() {
        let code = vec![15.0, 16.0, 17.0, 18.0, 19.0];
        let knowledge = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let (c, k) = detect_proportions(&code, &knowledge, 0.15);
        assert!((c + k - 1.0).abs() < 1e-10);
    }
}
