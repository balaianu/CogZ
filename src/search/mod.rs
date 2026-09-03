//! Search layer — hybrid FTS5 + vector search with RRF fusion and
//! graph expansion.
//!
//! The search module is model-agnostic: it takes an optional query
//! embedding (`Option<&[f32]>`) and falls back to FTS-only when none
//! is provided. Model loading and query embedding happen at the CLI
//! boundary, same pattern as `embed_synced`.

pub mod balance;
pub mod describe;
pub mod expand;
pub mod hybrid;
pub mod rrf;
pub mod scoring;

pub use expand::ExpansionResult;
pub use hybrid::search;
pub use rrf::fuse;
pub use scoring::{ScoreWeights, cold_start_score, recency_decay};

use crate::storage::crud::Entity;

/// A single search result with provenance.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entity: Entity,
    /// Fused relevance score from RRF. Graph-expanded results get a
    /// decayed score: `seed_relevance * 0.5^hops`.
    pub relevance: f32,
    /// Entity IDs tracing from the matched entity to this result.
    /// For direct matches: `[entity_id]`. For expanded results: the
    /// path from the seed to this entity.
    pub graph_path: Vec<String>,
    /// Human-readable description of the graph path.
    /// Empty for direct matches.
    pub graph_path_description: String,
}

/// How the search was executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// FTS5 + knowledge vector + code vector, fused via RRF.
    Hybrid,
    /// FTS5 + knowledge vector only (code model unavailable).
    KnowledgeHybrid,
    /// FTS5 + code vector only (knowledge model unavailable).
    CodeHybrid,
    /// FTS5 only (no embedding models available).
    FtsOnly,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::KnowledgeHybrid => "knowledge_hybrid",
            Self::CodeHybrid => "code_hybrid",
            Self::FtsOnly => "fts_only",
        }
    }
}

/// Query embeddings for dual-model search. Each is optional — when
/// absent, that vector channel is skipped (graceful degradation).
#[derive(Debug, Clone, Default)]
pub struct QueryEmbeddings<'a> {
    /// Knowledge-model query embedding (bge-base). Searches knowledge_embeddings.
    pub knowledge: Option<&'a [f32]>,
    /// Code-model query embedding (CodeRankEmbed). Searches code_embeddings.
    pub code: Option<&'a [f32]>,
}

impl<'a> QueryEmbeddings<'a> {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn knowledge(emb: &'a [f32]) -> Self {
        Self {
            knowledge: Some(emb),
            code: None,
        }
    }

    pub fn both(knowledge: &'a [f32], code: &'a [f32]) -> Self {
        Self {
            knowledge: Some(knowledge),
            code: Some(code),
        }
    }
}

/// Parameters for a search query.
#[derive(Debug, Clone)]
pub struct SearchParams {
    /// Filter by entity type. None = all types.
    pub entity_type: Option<String>,
    /// Filter by status. None defaults to "active". Use "all" for no filter.
    pub status: Option<String>,
    /// Max results before graph expansion.
    pub limit: u32,
    /// Whether to perform graph expansion from search results.
    pub expand: bool,
    /// Max graph hops for expansion (0 = no expansion even if expand=true).
    pub max_hops: usize,
    /// Whether to include test code entities in results. Test code
    /// (files under `tests/` or named `*_tests.rs` / `tests.rs`) is
    /// excluded by default to keep context packs focused on production
    /// code. Set to true to include test entities.
    pub include_tests: bool,
}

impl Default for SearchParams {
    fn default() -> Self {
        Self {
            entity_type: None,
            status: None,
            limit: 20,
            expand: true,
            max_hops: 2,
            include_tests: false,
        }
    }
}

/// Complete search results.
#[derive(Debug, Clone)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub search_mode: SearchMode,
}

/// Search-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
}
