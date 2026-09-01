//! ONNX Runtime NLI model for contradiction detection.
//!
//! Loads an NLI model (`cross-encoder/nli-deberta-v3-xsmall`) via `ort`
//! with dynamic linking. Degrades gracefully when the ONNX Runtime
//! library or model files are unavailable.

use std::path::PathBuf;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;
use tracing::warn;

use super::model::{EmbeddingError, EmbeddingResult, NliLabel, NliModel};
use super::registry;

/// Default NLI model ID.
const DEFAULT_NLI_MODEL: &str = registry::DEFAULT_NLI_MODEL;

/// ONNX Runtime NLI model for contradiction detection.
///
/// Loads the model and tokenizer lazily on first use. If the ONNX
/// Runtime library or model files are unavailable, `is_available()`
/// returns false and `classify` returns `EmbeddingError::ModelUnavailable`.
///
/// NLI models take a (premise, hypothesis) pair, tokenize them together
/// with a separator, and output 3 logits: entailment, neutral,
/// contradiction. The argmax determines the label.
pub struct OnnxNliModel {
    model_dir: PathBuf,
    session: Mutex<Option<Session>>,
    tokenizer: Mutex<Option<tokenizers::Tokenizer>>,
    available: Mutex<bool>,
}

impl OnnxNliModel {
    /// Create a new NLI model. `model_id` selects the model directory
    /// under `models_base`. Empty string uses the default
    /// (`Xenova/nli-deberta-v3-xsmall`).
    pub fn new(models_base: &std::path::Path, model_id: &str) -> Self {
        let id = if model_id.is_empty() {
            DEFAULT_NLI_MODEL
        } else {
            model_id
        };
        Self {
            model_dir: models_base.join(id),
            session: Mutex::new(None),
            tokenizer: Mutex::new(None),
            available: Mutex::new(false),
        }
    }

    /// Find the ONNX model file, trying all known layouts.
    /// Prefer quantized (smaller, faster) then full model.
    fn find_onnx_file(&self) -> Option<PathBuf> {
        [
            self.model_dir.join("onnx").join("model_quint8_avx2.onnx"),
            self.model_dir.join("onnx").join("model.onnx"),
            self.model_dir.join("model.onnx"),
        ]
        .into_iter()
        .find(|p| p.exists())
    }

    fn try_load(&self) -> EmbeddingResult<()> {
        if self.session.lock().unwrap().is_some() {
            return Ok(());
        }

        // Ensure the ONNX Runtime library is available.
        if !super::runtime::ensure_ort() {
            return Err(EmbeddingError::ModelUnavailable(
                "ONNX Runtime library not available".to_string(),
            ));
        }

        // Find the ONNX model file, trying all known layouts.
        // Prefer quantized (smaller, faster) then full model.
        let model_path = self.find_onnx_file();
        let Some(model_path) = model_path else {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "NLI model file not found under: {}",
                self.model_dir.display()
            )));
        };

        let tokenizer_path = self.model_dir.join("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "NLI tokenizer file not found: {}",
                tokenizer_path.display()
            )));
        }

        let session = {
            let mut builder = Session::builder().map_err(|e| {
                warn!("NLI session builder failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("session builder: {}", e))
            })?;
            builder = builder
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
                .map_err(|e| {
                    warn!("NLI optimization level failed: {}", e);
                    EmbeddingError::ModelUnavailable(format!("optimization level: {}", e))
                })?;
            builder.commit_from_file(&model_path).map_err(|e| {
                warn!("NLI session load failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("failed to load NLI model: {}", e))
            })?
        };

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            warn!("NLI tokenizer load failed: {}", e);
            EmbeddingError::ModelUnavailable(format!("failed to load NLI tokenizer: {}", e))
        })?;

        // Truncate to the model's max sequence length.
        let mut tokenizer = tokenizer;
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: 512,
                ..Default::default()
            }))
            .map_err(|e| {
                warn!("NLI tokenizer truncation setup failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("truncation setup: {}", e))
            })?;
        let tokenizer = tokenizer;

        *self.session.lock().unwrap() = Some(session);
        *self.tokenizer.lock().unwrap() = Some(tokenizer);
        *self.available.lock().unwrap() = true;
        Ok(())
    }

    /// Check if the model files exist on disk (without loading them).
    pub fn model_files_exist(&self) -> bool {
        let has_model = self.model_dir.join("model.onnx").exists()
            || self.model_dir.join("onnx").join("model.onnx").exists()
            || self
                .model_dir
                .join("onnx")
                .join("model_quint8_avx2.onnx")
                .exists();
        let has_tokenizer = self.model_dir.join("tokenizer.json").exists();
        has_model && has_tokenizer
    }
}

impl NliModel for OnnxNliModel {
    fn classify(&self, premise: &str, hypothesis: &str) -> EmbeddingResult<NliLabel> {
        self.try_load()?;

        let mut session_guard = self.session.lock().unwrap();
        let tokenizer_guard = self.tokenizer.lock().unwrap();
        let session: &mut Session = session_guard.as_mut().unwrap();
        let tokenizer = tokenizer_guard.as_ref().unwrap();

        // NLI models take a premise-hypothesis pair. The tokenizer
        // handles the pairing via EncodeInput::Dual (separator tokens).
        let encoded = tokenizer
            .encode((premise, hypothesis), true)
            .map_err(|e| EmbeddingError::InferenceFailed(format!("NLI tokenization: {}", e)))?;

        let input_ids: Vec<i64> = encoded.get_ids().iter().map(|&v| v as i64).collect();
        let attention_mask: Vec<i64> = encoded
            .get_attention_mask()
            .iter()
            .map(|&v| v as i64)
            .collect();

        let seq_len = input_ids.len() as i64;
        let batch = 1i64;

        let input_ids_tensor = Tensor::from_array((vec![batch, seq_len], input_ids))
            .map_err(|e| EmbeddingError::InferenceFailed(format!("NLI input tensor: {}", e)))?;
        let attention_mask_tensor =
            Tensor::from_array((vec![batch, seq_len], attention_mask.clone()))
                .map_err(|e| EmbeddingError::InferenceFailed(format!("NLI mask tensor: {}", e)))?;
        // DeBERTa-v3 cross-encoders accept 2 inputs only (no token_type_ids).
        // The quantized model declares it in the graph but rejects it at runtime.
        let outputs = session
            .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
            .map_err(|e| EmbeddingError::InferenceFailed(format!("NLI inference: {}", e)))?;

        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| EmbeddingError::InferenceFailed(format!("NLI extract: {}", e)))?;

        // Output shape: [1, 3] — logits for entailment, neutral, contradiction.
        // NLI label mapping for cross-encoder/nli-deberta-v3-xsmall: 0=contradiction, 1=entailment, 2=neutral.
        if data.len() < 3 {
            return Err(EmbeddingError::InferenceFailed(format!(
                "NLI output has {} values, expected 3",
                data.len()
            )));
        }

        let label = match (0..3)
            .map(|i| (i, data[i]))
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0)
        {
            0 => NliLabel::Contradiction,
            1 => NliLabel::Entailment,
            2 => NliLabel::Neutral,
            _ => NliLabel::Neutral,
        };

        Ok(label)
    }

    fn model_name(&self) -> &str {
        "nli-deberta-v3-xsmall"
    }

    fn is_available(&self) -> bool {
        *self.available.lock().unwrap()
    }
}
