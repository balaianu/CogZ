//! Embedding model trait and mock implementation.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Error type for embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("model not available: {0}")]
    ModelUnavailable(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
}

/// Result alias for embedding operations.
pub type EmbeddingResult<T> = std::result::Result<T, EmbeddingError>;

/// Trait for text embedding models.
///
/// Implementations include the ONNX Runtime model (`OnnxEmbeddingModel`)
/// and a deterministic mock (`MockEmbeddingModel`) for testing.
pub trait EmbeddingModel: Send + Sync {
    /// Embed a batch of texts, returning one vector per input.
    fn embed(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>>;

    /// Embedding dimension (e.g. 768 for CodeRankEmbed and bge-base).
    fn dimension(&self) -> usize;

    /// Model identifier for logging and cache keys.
    fn model_name(&self) -> &str;

    /// Whether the model is loaded and ready for inference.
    fn is_available(&self) -> bool;
}

/// Deterministic mock embedding model for tests.
///
/// Same text → same vector, different texts → different vectors
/// (with high probability). No network access, no ONNX dependency.
pub struct MockEmbeddingModel {
    dim: usize,
}

impl MockEmbeddingModel {
    pub fn new() -> Self {
        Self { dim: 768 }
    }

    pub fn with_dim(dim: usize) -> Self {
        Self { dim }
    }
}

impl Default for MockEmbeddingModel {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingModel for MockEmbeddingModel {
    fn embed(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut hasher = DefaultHasher::new();
                t.hash(&mut hasher);
                let h = hasher.finish();
                (0..self.dim)
                    .map(|i| ((h >> (i % 64)) & 1) as f32)
                    .collect()
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        "mock"
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embed_is_deterministic() {
        let model = MockEmbeddingModel::new();
        let v1 = model.embed(&["hello world"]).unwrap();
        let v2 = model.embed(&["hello world"]).unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn mock_embed_differs_for_different_text() {
        let model = MockEmbeddingModel::new();
        let v1 = model.embed(&["hello"]).unwrap();
        let v2 = model.embed(&["world"]).unwrap();
        assert_ne!(v1, v2);
    }

    #[test]
    fn mock_embed_correct_dimension() {
        let model = MockEmbeddingModel::with_dim(384);
        let v = model.embed(&["test"]).unwrap();
        assert_eq!(v[0].len(), 384);
    }

    #[test]
    fn mock_embed_batch() {
        let model = MockEmbeddingModel::new();
        let vs = model.embed(&["a", "b", "c"]).unwrap();
        assert_eq!(vs.len(), 3);
        assert_eq!(vs[0].len(), 768);
    }

    #[test]
    fn mock_is_available() {
        let model = MockEmbeddingModel::new();
        assert!(model.is_available());
    }
}
