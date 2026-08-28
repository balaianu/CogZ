//! Embedding layer — ONNX model inference and content-hash caching.
//!
//! Provides the `EmbeddingModel` trait, an ONNX Runtime implementation,
//! and a content-hash cache that avoids re-embedding unchanged text.
//! Degrades gracefully when models are unavailable (FTS-only search).

pub mod cache;
pub mod model;
pub mod onnx;

pub use cache::EmbeddingCache;
pub use model::{EmbeddingModel, EmbeddingResult, MockEmbeddingModel};
pub use onnx::{ModelType, OnnxEmbeddingModel};

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
