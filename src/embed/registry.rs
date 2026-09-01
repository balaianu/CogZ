//! Model registry — maps logical model names to actual HuggingFace
//! download sources.
//!
//! CogZ-py discovered that the canonical model names (e.g.
//! `nomic-ai/CodeRankEmbed-int8`) are often gated or lack ONNX files.
//! The solution is to download from community ONNX export repos that
//! are ungated and provide pre-optimized/quantized ONNX files.
//!
//! This registry mirrors CogZ-py's `_CUSTOM_MODELS` mapping, adapted
//! for the Rust implementation.

/// Which ONNX file to download for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnnxLayout {
    /// `model.onnx` at the repo root (Qdrant's `all-MiniLM-L6-v2-onnx`).
    RootModel,
    /// `model_optimized.onnx` at the repo root (Qdrant's `-onnx-Q` repos).
    RootOptimized,
    /// `onnx/model.onnx` in a subdirectory (Xenova, cross-encoder exports).
    OnnxSubdir,
    /// `onnx/model_quint8_avx2.onnx` — INT8 quantized for AVX2 CPUs.
    OnnxSubdirQuantizedAvx2,
}

/// A model entry in the registry.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    /// The HuggingFace repo to download from (e.g. `mrsladoje/CodeRankEmbed-onnx-int8`).
    pub hf_source: &'static str,
    /// Where the ONNX file lives in the repo.
    pub onnx_layout: OnnxLayout,
    /// Embedding dimension (must match config).
    pub dim: usize,
    /// Model size in MB (for progress reporting).
    pub size_mb: usize,
}

/// The logical model kind — determines which registry entry to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Code,
    Knowledge,
    Nli,
}

impl ModelKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Knowledge => "knowledge",
            Self::Nli => "nli",
        }
    }
}

/// Default model IDs — the values written into `config.toml` by
/// `cogz init`. These are the logical names users see. The registry
/// maps them to actual download sources.
pub const DEFAULT_CODE_MODEL: &str = "nomic-ai/CodeRankEmbed-int8";
pub const DEFAULT_KNOWLEDGE_MODEL: &str = "BAAI/bge-base-en-v1.5";
pub const DEFAULT_NLI_MODEL: &str = "cross-encoder/nli-deberta-v3-xsmall";
pub const DEFAULT_DIMENSION: usize = 768;

/// Look up a model ID in the registry. Returns the actual HF source
/// repo and ONNX file layout.
///
/// If the model ID is not in the registry, returns None — the caller
/// should treat it as a direct HF repo ID with `OnnxLayout::OnnxSubdir`
/// (the most common layout for community ONNX exports).
pub fn lookup(model_id: &str) -> Option<ModelEntry> {
    match model_id {
        // Code: INT8 quantized CodeRankEmbed — 139MB, 768d, code-specific.
        // Proven in CogZ-py: 74% less RAM, 38% faster, cosine ≥ 0.96 vs FP32.
        "nomic-ai/CodeRankEmbed-int8" => Some(ModelEntry {
            hf_source: "mrsladoje/CodeRankEmbed-onnx-int8",
            onnx_layout: OnnxLayout::OnnxSubdir,
            dim: 768,
            size_mb: 139,
        }),

        // Code: full-precision CodeRankEmbed — 548MB, 768d.
        "nomic-ai/CodeRankEmbed" => Some(ModelEntry {
            hf_source: "jamie8johnson/CodeRankEmbed-onnx",
            onnx_layout: OnnxLayout::OnnxSubdir,
            dim: 768,
            size_mb: 548,
        }),

        // Knowledge: Qdrant's graph-optimized bge-base — 210MB, 768d.
        // Uses model_optimized.onnx with graph optimizations baked in.
        "BAAI/bge-base-en-v1.5" => Some(ModelEntry {
            hf_source: "Qdrant/bge-base-en-v1.5-onnx-Q",
            onnx_layout: OnnxLayout::RootOptimized,
            dim: 768,
            size_mb: 210,
        }),

        // Knowledge: Qdrant's bge-small — 64MB, 384d. Lighter alternative.
        "BAAI/bge-small-en-v1.5" => Some(ModelEntry {
            hf_source: "Qdrant/bge-small-en-v1.5-onnx-Q",
            onnx_layout: OnnxLayout::RootOptimized,
            dim: 384,
            size_mb: 64,
        }),

        // NLI: cross-encoder deberta-v3-xsmall — 284MB full, 87MB quantized.
        // We use the quint8 AVX2 quantized variant for 3x smaller size.
        "cross-encoder/nli-deberta-v3-xsmall" => Some(ModelEntry {
            hf_source: "cross-encoder/nli-deberta-v3-xsmall",
            onnx_layout: OnnxLayout::OnnxSubdirQuantizedAvx2,
            dim: 768,
            size_mb: 87,
        }),

        // Fallback: Xenova NLI export (if someone has it cached).
        "Xenova/nli-deberta-v3-xsmall" => Some(ModelEntry {
            hf_source: "Xenova/nli-deberta-v3-xsmall",
            onnx_layout: OnnxLayout::OnnxSubdir,
            dim: 768,
            size_mb: 284,
        }),

        _ => None,
    }
}

/// Resolve a model ID to its HF download source. If the model is in
/// the registry, uses the mapped source. Otherwise, uses the model ID
/// directly (assuming it's a valid HF repo with `onnx/model.onnx`).
pub fn resolve_source(model_id: &str) -> (&'static str, OnnxLayout) {
    if let Some(entry) = lookup(model_id) {
        (entry.hf_source, entry.onnx_layout)
    } else {
        // Unknown model — try as direct repo with onnx/model.onnx layout.
        // Leak the string to get a 'static lifetime (this is called once
        // per model load, not in a hot loop).
        let leaked: &'static str = Box::leak(model_id.to_string().into_boxed_str());
        (leaked, OnnxLayout::OnnxSubdir)
    }
}

/// The ONNX filename to download for a given layout.
pub fn onnx_filename(layout: OnnxLayout) -> &'static str {
    match layout {
        OnnxLayout::RootModel => "model.onnx",
        OnnxLayout::RootOptimized => "model_optimized.onnx",
        OnnxLayout::OnnxSubdir => "onnx/model.onnx",
        OnnxLayout::OnnxSubdirQuantizedAvx2 => "onnx/model_quint8_avx2.onnx",
    }
}

/// The ONNX file path relative to the snapshot root, for loader use.
pub fn onnx_relative_path(layout: OnnxLayout) -> &'static str {
    match layout {
        OnnxLayout::RootModel => "model.onnx",
        OnnxLayout::RootOptimized => "model_optimized.onnx",
        OnnxLayout::OnnxSubdir => "onnx/model.onnx",
        OnnxLayout::OnnxSubdirQuantizedAvx2 => "onnx/model_quint8_avx2.onnx",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_maps_default_code_model() {
        let entry = lookup(DEFAULT_CODE_MODEL).unwrap();
        assert_eq!(entry.hf_source, "mrsladoje/CodeRankEmbed-onnx-int8");
        assert_eq!(entry.onnx_layout, OnnxLayout::OnnxSubdir);
        assert_eq!(entry.dim, 768);
    }

    #[test]
    fn registry_maps_default_knowledge_model() {
        let entry = lookup(DEFAULT_KNOWLEDGE_MODEL).unwrap();
        assert_eq!(entry.hf_source, "Qdrant/bge-base-en-v1.5-onnx-Q");
        assert_eq!(entry.onnx_layout, OnnxLayout::RootOptimized);
        assert_eq!(entry.dim, 768);
    }

    #[test]
    fn registry_maps_coderank_int8() {
        let entry = lookup("nomic-ai/CodeRankEmbed-int8").unwrap();
        assert_eq!(entry.hf_source, "mrsladoje/CodeRankEmbed-onnx-int8");
        assert_eq!(entry.onnx_layout, OnnxLayout::OnnxSubdir);
        assert_eq!(entry.dim, 768);
    }

    #[test]
    fn registry_maps_bge_base() {
        let entry = lookup("BAAI/bge-base-en-v1.5").unwrap();
        assert_eq!(entry.hf_source, "Qdrant/bge-base-en-v1.5-onnx-Q");
        assert_eq!(entry.onnx_layout, OnnxLayout::RootOptimized);
        assert_eq!(entry.dim, 768);
    }

    #[test]
    fn registry_maps_nli() {
        let entry = lookup(DEFAULT_NLI_MODEL).unwrap();
        assert_eq!(entry.hf_source, "cross-encoder/nli-deberta-v3-xsmall");
        assert_eq!(entry.onnx_layout, OnnxLayout::OnnxSubdirQuantizedAvx2);
    }

    #[test]
    fn unknown_model_falls_back_to_direct() {
        let (source, layout) = resolve_source("some/custom/model");
        assert_eq!(source, "some/custom/model");
        assert_eq!(layout, OnnxLayout::OnnxSubdir);
    }

    #[test]
    fn onnx_filename_matches_layout() {
        assert_eq!(
            onnx_filename(OnnxLayout::RootOptimized),
            "model_optimized.onnx"
        );
        assert_eq!(onnx_filename(OnnxLayout::OnnxSubdir), "onnx/model.onnx");
        assert_eq!(
            onnx_filename(OnnxLayout::OnnxSubdirQuantizedAvx2),
            "onnx/model_quint8_avx2.onnx"
        );
    }
}
