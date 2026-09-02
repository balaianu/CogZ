//! ONNX Runtime embedding model implementation.
//!
//! Loads ONNX models (CodeRankEmbed for code, bge-base for knowledge)
//! via `ort` with dynamic linking. Degrades gracefully when the ONNX
//! Runtime library or model files are unavailable.

use std::path::PathBuf;
use std::sync::Mutex;

use ort::session::Session;
use tracing::warn;

use super::inference::run_inference;
use super::model::{EmbeddingError, EmbeddingModel, EmbeddingResult};
use super::model_type::ModelType;

/// ONNX Runtime embedding model.
///
/// Loads the ONNX model and tokenizer lazily on first use. If the
/// ONNX Runtime library (`libonnxruntime.so`) is not found, or the
/// model files are missing, `is_available()` returns false and `embed`
/// returns `EmbeddingError::ModelUnavailable`.
pub struct OnnxEmbeddingModel {
    model_type: ModelType,
    model_dir: PathBuf,
    model_id: String,
    dimension: usize,
    session: Mutex<Option<Session>>,
    tokenizer: Mutex<Option<tokenizers::Tokenizer>>,
    available: Mutex<bool>,
    has_token_type_ids: Mutex<bool>,
    idle_tracker: super::resources::IdleTracker,
    min_free_mb: u64,
}

impl OnnxEmbeddingModel {
    /// Create a new ONNX embedding model. Does not load the model yet —
    /// that happens on first `embed()` call (lazy loading).
    ///
    /// Uses the hardcoded model ID for `model_type` to locate the
    /// model directory under `models_base`.
    pub fn new(model_type: ModelType, models_base: &std::path::Path, dimension: usize) -> Self {
        let model_id = model_type.default_model_id().to_string();
        Self {
            model_type,
            model_dir: model_type.model_dir(models_base),
            model_id,
            dimension,
            session: Mutex::new(None),
            tokenizer: Mutex::new(None),
            available: Mutex::new(false),
            has_token_type_ids: Mutex::new(true),
            idle_tracker: super::resources::IdleTracker::new(0),
            min_free_mb: 0,
        }
    }

    /// Create a new ONNX embedding model with a config-specified model ID.
    ///
    /// When `model_id` is non-empty, the model directory is
    /// `models_base/{model_id}` instead of the hardcoded default.
    /// This wires `[embedding].code_model` and `[embedding].knowledge_model`
    /// to actual model selection.
    pub fn with_model_id(
        model_type: ModelType,
        models_base: &std::path::Path,
        dimension: usize,
        model_id: &str,
    ) -> Self {
        Self::with_resource_config(model_type, models_base, dimension, model_id, 0, 0)
    }

    /// Create with resource-aware settings (idle TTL + memory floor).
    pub fn with_resource_config(
        model_type: ModelType,
        models_base: &std::path::Path,
        dimension: usize,
        model_id: &str,
        idle_ttl_secs: u64,
        min_free_mb: u64,
    ) -> Self {
        let (model_dir, resolved_id) = if model_id.is_empty() {
            (
                model_type.model_dir(models_base),
                model_type.default_model_id().to_string(),
            )
        } else {
            (models_base.join(model_id), model_id.to_string())
        };
        Self {
            model_type,
            model_dir,
            model_id: resolved_id,
            dimension,
            session: Mutex::new(None),
            tokenizer: Mutex::new(None),
            available: Mutex::new(false),
            has_token_type_ids: Mutex::new(true),
            idle_tracker: super::resources::IdleTracker::new(idle_ttl_secs),
            min_free_mb,
        }
    }

    /// Try to load the model and tokenizer.
    fn try_load(&self) -> EmbeddingResult<()> {
        // Unload idle model to free memory before reloading.
        if self.idle_tracker.is_idle() {
            self.unload();
        }

        if self.session.lock().unwrap().is_some() {
            self.idle_tracker.touch();
            return Ok(());
        }

        if !super::resources::has_enough_memory(self.min_free_mb) {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "insufficient free memory (need >= {} MB)",
                self.min_free_mb
            )));
        }

        // Ensure the ONNX Runtime library is available and initialized.
        // This discovers or downloads the ORT shared library and calls
        // ort::init_from() to point ort to it.
        if !super::runtime::ensure_ort() {
            return Err(EmbeddingError::ModelUnavailable(
                "ONNX Runtime library not available".to_string(),
            ));
        }

        // Clean broken cache artifacts before loading. The cache root
        // is the parent of the model directory.
        if let Some(cache_root) = self.model_dir.parent() {
            super::download::clean_broken_cache(cache_root);
        }

        // Find the ONNX model file. The registry knows which layout
        // each model uses (root, onnx subdir, optimized, quantized).
        let model_path = self.find_onnx_file();
        let Some(model_path) = model_path else {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "model file not found under: {}",
                self.model_dir.display()
            )));
        };

        let tokenizer_path = self.model_dir.join("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(EmbeddingError::ModelUnavailable(format!(
                "tokenizer file not found: {}",
                tokenizer_path.display()
            )));
        }

        let session = {
            let mut builder = Session::builder().map_err(|e| {
                warn!("ONNX session builder failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("session builder: {}", e))
            })?;
            builder = builder
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
                .map_err(|e| {
                    warn!("ONNX optimization level failed: {}", e);
                    EmbeddingError::ModelUnavailable(format!("optimization level: {}", e))
                })?;
            // Disable memory pattern to prevent ORT's arena from
            // growing unbounded across batched inference calls. Without
            // this, RSS climbs to 5GB+ during large embedding workloads.
            builder = builder.with_memory_pattern(false).map_err(|e| {
                warn!("ONNX memory pattern failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("memory pattern: {}", e))
            })?;
            builder.commit_from_file(&model_path).map_err(|e| {
                warn!("ONNX session load failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("failed to load ONNX model: {}", e))
            })?
        };

        // Check whether the model declares token_type_ids as an input.
        // CodeRankEmbed does not; BERT-based models do. Passing an
        // extra input the model doesn't accept causes a runtime error.
        let has_tti = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        *self.has_token_type_ids.lock().unwrap() = has_tti;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            warn!("tokenizer load failed: {}", e);
            EmbeddingError::ModelUnavailable(format!("failed to load tokenizer: {}", e))
        })?;

        // Truncate to 256 tokens. BERT-based models support 512, but
        // 256 captures sufficient semantic information for code/knowledge
        // embedding while halving inference time for long sequences.
        // Also set padding to the longest in the batch — the tokenizer
        // handles padding automatically, which is much faster than
        // manual padding in Rust.
        let mut tokenizer = tokenizer;
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: 256,
                ..Default::default()
            }))
            .map_err(|e| {
                warn!("tokenizer truncation setup failed: {}", e);
                EmbeddingError::ModelUnavailable(format!("truncation setup: {}", e))
            })?;
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            ..Default::default()
        }));
        let tokenizer = tokenizer;

        *self.session.lock().unwrap() = Some(session);
        *self.tokenizer.lock().unwrap() = Some(tokenizer);
        *self.available.lock().unwrap() = true;
        self.idle_tracker.touch();
        Ok(())
    }

    /// Drop the loaded model and tokenizer, freeing memory.
    pub fn unload(&self) {
        self.session.lock().unwrap().take();
        self.tokenizer.lock().unwrap().take();
        *self.available.lock().unwrap() = false;
    }

    /// Find the ONNX model file in the model directory, trying all
    /// known layouts: root model.onnx, root model_optimized.onnx,
    /// onnx/model.onnx, onnx/model_quantized.onnx.
    fn find_onnx_file(&self) -> Option<PathBuf> {
        [
            self.model_dir.join("model_optimized.onnx"),
            self.model_dir.join("model.onnx"),
            self.model_dir.join("onnx").join("model_quantized.onnx"),
            self.model_dir.join("onnx").join("model.onnx"),
        ]
        .into_iter()
        .find(|p| p.exists())
    }

    /// Check if the model files exist on disk (without loading them).
    pub fn model_files_exist(&self) -> bool {
        self.find_onnx_file().is_some() && self.model_dir.join("tokenizer.json").exists()
    }

    /// The query prefix for instruction-aware models. CodeRankEmbed
    /// requires "Represent this query for searching relevant code: "
    /// prepended to search queries (not documents). Other models
    /// return an empty string.
    fn query_prefix(&self) -> &str {
        if self.model_id.contains("CodeRankEmbed") {
            "Represent this query for searching relevant code: "
        } else {
            ""
        }
    }
}

impl EmbeddingModel for OnnxEmbeddingModel {
    fn embed(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        self.try_load()?;

        // Process in chunks to avoid excessive memory use on large
        // batches. 32 balances throughput and memory for 512-token
        // sequences with 768-dim embeddings. Larger batches (64+)
        // cause ORT's memory arena to grow beyond 5GB RSS.
        const CHUNK_SIZE: usize = 32;
        let mut all_results = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(CHUNK_SIZE) {
            let embeddings = self.embed_chunk(chunk)?;
            all_results.extend(embeddings);
        }

        Ok(all_results)
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

    fn embed_query(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        let prefix = self.query_prefix();
        if prefix.is_empty() {
            return self.embed(texts);
        }
        let prefixed: Vec<String> = texts.iter().map(|t| format!("{}{}", prefix, t)).collect();
        let refs: Vec<&str> = prefixed.iter().map(|s| s.as_str()).collect();
        self.embed(&refs)
    }
}

impl OnnxEmbeddingModel {
    /// Embed a single chunk of texts (≤ CHUNK_SIZE). Holds the session
    /// and tokenizer locks for the duration of inference.
    fn embed_chunk(&self, texts: &[&str]) -> EmbeddingResult<Vec<Vec<f32>>> {
        let mut session_guard = self.session.lock().unwrap();
        let tokenizer_guard = self.tokenizer.lock().unwrap();
        let session = session_guard.as_mut().unwrap();
        let tokenizer = tokenizer_guard.as_ref().unwrap();
        let has_tti = *self.has_token_type_ids.lock().unwrap();

        run_inference(session, tokenizer, texts, self.dimension, has_tti)
    }
}

#[cfg(test)]
#[path = "onnx_tests.rs"]
mod tests;
