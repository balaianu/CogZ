//! ONNX Runtime embedding model implementation.
//!
//! Loads ONNX models (CodeRankEmbed for code, bge-base for knowledge)
//! via `ort` with dynamic linking. Degrades gracefully when the ONNX
//! Runtime library or model files are unavailable.

use std::path::PathBuf;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;
use tracing::warn;

use super::model::{EmbeddingError, EmbeddingModel, EmbeddingResult};

/// Which embedding model to use — determines the model file path
/// and tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelType {
    /// `nomic-ai/CodeRankEmbed-int8` — code-specific embeddings.
    Code,
    /// `BAAI/bge-base-en-v1.5` — general-purpose embeddings.
    Knowledge,
}

impl ModelType {
    pub fn model_id(&self) -> &'static str {
        match self {
            Self::Code => "nomic-ai/CodeRankEmbed-int8",
            Self::Knowledge => "BAAI/bge-base-en-v1.5",
        }
    }

    pub fn model_name(&self) -> &'static str {
        match self {
            Self::Code => "coderank-embed",
            Self::Knowledge => "bge-base",
        }
    }

    /// Local path for the downloaded model directory.
    pub fn model_dir(&self, base: &std::path::Path) -> PathBuf {
        base.join(self.model_id())
    }
}

/// ONNX Runtime embedding model.
///
/// Loads the ONNX model and tokenizer lazily on first use. If the
/// ONNX Runtime library (`libonnxruntime.so`) is not found, or the
/// model files are missing, `is_available()` returns false and `embed`
/// returns `EmbeddingError::ModelUnavailable`.
pub struct OnnxEmbeddingModel {
    model_type: ModelType,
    model_dir: PathBuf,
    dimension: usize,
    session: Mutex<Option<Session>>,
    tokenizer: Mutex<Option<tokenizers::Tokenizer>>,
    available: Mutex<bool>,
}

impl OnnxEmbeddingModel {
    /// Create a new ONNX embedding model. Does not load the model yet —
    /// that happens on first `embed()` call (lazy loading).
    pub fn new(model_type: ModelType, models_base: &std::path::Path, dimension: usize) -> Self {
        Self {
            model_type,
            model_dir: model_type.model_dir(models_base),
            dimension,
            session: Mutex::new(None),
            tokenizer: Mutex::new(None),
            available: Mutex::new(false),
        }
    }

    /// Try to load the model and tokenizer.
    fn try_load(&self) -> EmbeddingResult<()> {
        if self.session.lock().unwrap().is_some() {
            return Ok(());
        }

        let model_path = self.model_dir.join("model.onnx");
        let tokenizer_path = self.model_dir.join("tokenizer.json");

        if !model_path.exists() {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "model file not found: {}",
                model_path.display()
            )));
        }
        if !tokenizer_path.exists() {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "tokenizer file not found: {}",
                tokenizer_path.display()
            )));
        }

        let session = Session::builder()
            .and_then(|b| b.commit_from_file(&model_path))
            .map_err(|e| {
                warn!("ONNX session load failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("failed to load ONNX model: {}", e))
            })?;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            warn!("tokenizer load failed: {}", e);
            EmbeddingError::ModelUnavailable(format!("failed to load tokenizer: {}", e))
        })?;

        *self.session.lock().unwrap() = Some(session);
        *self.tokenizer.lock().unwrap() = Some(tokenizer);
        *self.available.lock().unwrap() = true;
        Ok(())
    }

    /// Check if the model files exist on disk (without loading them).
    pub fn model_files_exist(&self) -> bool {
        self.model_dir.join("model.onnx").exists() && self.model_dir.join("tokenizer.json").exists()
    }
}

impl EmbeddingModel for OnnxEmbeddingModel {
    fn embed(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        self.try_load()?;

        let mut session_guard = self.session.lock().unwrap();
        let tokenizer_guard = self.tokenizer.lock().unwrap();
        let session = session_guard.as_mut().unwrap();
        let tokenizer = tokenizer_guard.as_ref().unwrap();

        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            let encoded = tokenizer
                .encode(*text, true)
                .map_err(|e| EmbeddingError::InferenceFailed(format!("tokenization: {}", e)))?;

            let input_ids: Vec<i64> = encoded.get_ids().iter().map(|&v| v as i64).collect();
            let attention_mask: Vec<i64> = encoded
                .get_attention_mask()
                .iter()
                .map(|&v| v as i64)
                .collect();
            let seq_len = input_ids.len() as i64;

            let input_ids_tensor = Tensor::from_array((vec![1, seq_len], input_ids))
                .map_err(|e| EmbeddingError::InferenceFailed(format!("input tensor: {}", e)))?;
            let attention_mask_tensor =
                Tensor::from_array((vec![1, seq_len], attention_mask.clone()))
                    .map_err(|e| EmbeddingError::InferenceFailed(format!("mask tensor: {}", e)))?;

            let outputs = session
                .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
                .map_err(|e| EmbeddingError::InferenceFailed(format!("inference: {}", e)))?;

            // Extract embedding from the first output. Shape is either
            // [1, seq_len, dim] (mean-pool) or [1, dim] (already pooled).
            let (shape, data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| EmbeddingError::InferenceFailed(format!("extract: {}", e)))?;

            let embedding = if shape.len() == 3 {
                let dim = shape[2] as usize;
                let seq = shape[1] as usize;
                mean_pool_with_mask(data, &attention_mask, seq, dim)
            } else if shape.len() == 2 {
                let dim = shape[1] as usize;
                data[..dim].to_vec()
            } else {
                return Err(EmbeddingError::InferenceFailed(format!(
                    "unexpected output rank: {}",
                    shape.len()
                )));
            };

            if embedding.len() != self.dimension {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.dimension,
                    actual: embedding.len(),
                });
            }

            results.push(embedding);
        }

        Ok(results)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        self.model_type.model_name()
    }

    fn is_available(&self) -> bool {
        *self.available.lock().unwrap()
    }
}

/// Mean-pool a rank-3 model output `[seq, dim]` using the attention
/// mask to exclude padding tokens. Padding positions (mask == 0)
/// contribute nothing to the sum and are not counted in the divisor.
///
/// Falls back to dividing by `seq` if the mask is all-zero (should
/// not happen with valid input but avoids division by zero).
pub fn mean_pool_with_mask(
    data: &[f32],
    attention_mask: &[i64],
    seq: usize,
    dim: usize,
) -> Vec<f32> {
    let mut pooled = vec![0.0_f32; dim];
    let mut mask_sum = 0.0_f32;
    for s in 0..seq {
        if s < attention_mask.len() && attention_mask[s] == 0 {
            continue;
        }
        mask_sum += 1.0;
        let offset = s * dim;
        for d in 0..dim {
            pooled[d] += data[offset + d];
        }
    }
    let divisor = if mask_sum > 0.0 { mask_sum } else { seq as f32 };
    for v in &mut pooled {
        *v /= divisor;
    }
    pooled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pool_all_ones_mask_averages_all_tokens() {
        // 3 tokens, 2 dims: [[1,2],[3,4],[5,6]]
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mask = vec![1, 1, 1];
        let pooled = mean_pool_with_mask(&data, &mask, 3, 2);
        assert_eq!(pooled, vec![3.0, 4.0]); // (1+3+5)/3, (2+4+6)/3
    }

    #[test]
    fn mean_pool_excludes_padding_tokens() {
        // 3 tokens, 2 dims, last token is padding
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mask = vec![1, 1, 0];
        let pooled = mean_pool_with_mask(&data, &mask, 3, 2);
        assert_eq!(pooled, vec![2.0, 3.0]); // (1+3)/2, (2+4)/2
    }

    #[test]
    fn mean_pool_single_token() {
        let data = vec![1.0, 2.0, 3.0];
        let mask = vec![1];
        let pooled = mean_pool_with_mask(&data, &mask, 1, 3);
        assert_eq!(pooled, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn mean_pool_all_padding_falls_back_to_seq_divisor() {
        // All-zero mask → all tokens skipped, sum stays 0.
        // Fallback to seq divisor avoids division by zero.
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let mask = vec![0, 0];
        let pooled = mean_pool_with_mask(&data, &mask, 2, 2);
        assert_eq!(pooled, vec![0.0, 0.0]); // 0/2, 0/2
    }

    #[test]
    fn mean_pool_mask_shorter_than_seq_uses_seq_for_overflow() {
        // Mask shorter than seq: overflow positions are treated as
        // active (mask_sum counts them) — defensive for misaligned
        // mask lengths.
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let mask = vec![1]; // only 1 entry, seq=2
        let pooled = mean_pool_with_mask(&data, &mask, 2, 2);
        // Token 0: mask=1, token 1: no mask entry → included
        assert_eq!(pooled, vec![2.0, 3.0]); // (1+3)/2, (2+4)/2
    }
}
