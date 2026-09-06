//! Similarity metrics for embedding vectors.

/// Cosine similarity between two vectors. Returns 0.0 if either
/// vector has zero norm. Range: [-1.0, 1.0], where 1.0 = identical
/// direction, 0.0 = orthogonal, -1.0 = opposite.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
}

/// Convert an L2 distance between L2-normalized vectors to cosine
/// similarity. For unit vectors, `cos = 1 - L2² / 2`.
pub fn l2_to_cosine(distance: f64) -> f64 {
    1.0 - (distance * distance) / 2.0
}
