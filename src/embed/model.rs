//! Embedding and NLI model traits and mock implementations.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Error type for embedding and NLI operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("model not available: {0}")]
    ModelUnavailable(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
}

/// Result alias for embedding and NLI operations.
pub type EmbeddingResult<T> = std::result::Result<T, EmbeddingError>;

/// Trait for text embedding models.
///
/// Implementations include the ONNX Runtime model (`OnnxEmbeddingModel`)
/// and a deterministic mock (`MockEmbeddingModel`) for testing.
pub trait EmbeddingModel: Send + Sync {
    /// Embed a batch of texts, returning one vector per input.
    fn embed(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>>;

    /// Embedding dimension (e.g. 768 for nomic-embed and bge-base).
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

/// NLI classification label — the result of comparing a premise
/// against a hypothesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NliLabel {
    /// The hypothesis follows from the premise.
    Entailment,
    /// The hypothesis is unrelated to the premise.
    Neutral,
    /// The hypothesis contradicts the premise.
    Contradiction,
}

impl NliLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Entailment => "entailment",
            Self::Neutral => "neutral",
            Self::Contradiction => "contradiction",
        }
    }
}

/// Trait for natural language inference (NLI) models.
///
/// Used by consolidation to detect contradictions between observations
/// and rules. Implementations include the ONNX Runtime model
/// (`OnnxNliModel`) and a deterministic mock (`MockNliModel`) for
/// testing.
pub trait NliModel: Send + Sync {
    /// Classify the relationship between a premise and a hypothesis.
    fn classify(&self, premise: &str, hypothesis: &str) -> EmbeddingResult<NliLabel>;

    /// Model identifier for logging.
    fn model_name(&self) -> &str;

    /// Whether the model is loaded and ready for inference.
    fn is_available(&self) -> bool;
}

/// Deterministic mock NLI model for tests.
///
/// Classifies based on simple keyword matching: if the hypothesis
/// contains a negation word ("not", "never", "wrong", "incorrect",
/// "actually") and the premise doesn't, it's a contradiction. If the
/// texts are identical or near-identical, it's entailment. Otherwise
/// neutral. No network access, no ONNX dependency.
pub struct MockNliModel;

impl NliModel for MockNliModel {
    fn classify(&self, premise: &str, hypothesis: &str) -> EmbeddingResult<NliLabel> {
        let p_lower = premise.to_lowercase();
        let h_lower = hypothesis.to_lowercase();

        if p_lower == h_lower {
            return Ok(NliLabel::Entailment);
        }

        let p_neg = has_negation(&p_lower);
        let h_neg = has_negation(&h_lower);

        if p_neg != h_neg {
            return Ok(NliLabel::Contradiction);
        }

        Ok(NliLabel::Neutral)
    }

    fn model_name(&self) -> &str {
        "mock-nli"
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn has_negation(text: &str) -> bool {
    let negations = [
        " not ",
        " never ",
        " wrong",
        " incorrect",
        " actually",
        "no ",
    ];
    negations.iter().any(|n| text.contains(n))
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

    #[test]
    fn mock_nli_entailment_for_identical_text() {
        let model = MockNliModel;
        let label = model
            .classify("The bug is in search", "The bug is in search")
            .unwrap();
        assert_eq!(label, NliLabel::Entailment);
    }

    #[test]
    fn mock_nli_contradiction_for_negated_hypothesis() {
        let model = MockNliModel;
        let label = model
            .classify("The bug is in search", "The bug is not in search")
            .unwrap();
        assert_eq!(label, NliLabel::Contradiction);
    }

    #[test]
    fn mock_nli_neutral_for_unrelated() {
        let model = MockNliModel;
        let label = model
            .classify("The bug is in search", "The config file is missing")
            .unwrap();
        assert_eq!(label, NliLabel::Neutral);
    }

    #[test]
    fn mock_nli_is_available() {
        let model = MockNliModel;
        assert!(model.is_available());
    }

    #[test]
    fn nli_label_as_str() {
        assert_eq!(NliLabel::Entailment.as_str(), "entailment");
        assert_eq!(NliLabel::Neutral.as_str(), "neutral");
        assert_eq!(NliLabel::Contradiction.as_str(), "contradiction");
    }
}
