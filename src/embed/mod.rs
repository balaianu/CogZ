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
