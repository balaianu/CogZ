//! Embedding layer — ONNX model inference and content-hash caching.
//!
//! Provides the `EmbeddingModel` trait, an ONNX Runtime implementation,
//! and a content-hash cache that avoids re-embedding unchanged text.
//! Degrades gracefully when models are unavailable (FTS-only search).

pub mod cache;
pub mod download;
pub mod inference;
pub mod model;
pub mod model_type;
pub mod nli;
pub mod onnx;
pub mod pooling;
pub mod registry;
pub mod resources;
pub mod runtime;
pub mod similarity;
pub mod suppress;

pub use cache::EmbeddingCache;
pub use download::{
    DownloadError, DownloadedModel, clean_broken_cache, download_model, is_model_cached,
};
pub use model::{
    EmbeddingModel, EmbeddingResult, MockEmbeddingModel, MockNliModel, NliLabel, NliModel,
    NliProbabilities,
};
pub use model_type::ModelType;
pub use nli::OnnxNliModel;
pub use onnx::OnnxEmbeddingModel;
pub use registry::{ModelKind, lookup, onnx_filename, onnx_relative_path, resolve_source};
pub use runtime::ensure_ort;

/// Get the models directory: `~/.local/share/cogz/models/` on Linux,
/// `%LOCALAPPDATA%/cogz/models/` on Windows. Falls back to
/// `.cogz/models` if no data directory is found.
pub fn models_dir() -> std::path::PathBuf {
    if let Some(dir) = data_dir() {
        dir.join("cogz").join("models")
    } else {
        std::path::PathBuf::from(".cogz/models")
    }
}

fn data_dir() -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        if let Ok(home) = std::env::var("HOME") {
            return Some(std::path::PathBuf::from(home).join(".local/share"));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("LOCALAPPDATA") {
            return Some(std::path::PathBuf::from(appdata));
        }
    }
    None
}
