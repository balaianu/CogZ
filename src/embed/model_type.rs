//! Embedding model type — code vs knowledge, with model ID resolution.

use std::path::PathBuf;

use super::registry;

/// Which embedding model to use — determines the model file path
/// and tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelType {
    /// Code embedding model (CodeRankEmbed-int8).
    Code,
    /// Knowledge embedding model (bge-base-en-v1.5).
    Knowledge,
}

impl ModelType {
    pub fn default_model_id(&self) -> &'static str {
        match self {
            Self::Code => registry::DEFAULT_CODE_MODEL,
            Self::Knowledge => registry::DEFAULT_KNOWLEDGE_MODEL,
        }
    }

    pub fn model_name(&self) -> &'static str {
        match self {
            Self::Code => "coderankembed",
            Self::Knowledge => "bge-base",
        }
    }

    /// Local path for the downloaded model directory.
    pub fn model_dir(&self, base: &std::path::Path) -> PathBuf {
        base.join(self.default_model_id())
    }
}
