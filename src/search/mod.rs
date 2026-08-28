//! Search layer — hybrid FTS5 + vector search with RRF fusion and
//! graph expansion.
//!
//! The search module is model-agnostic: it takes an optional query
//! embedding (`Option<&[f32]>`) and falls back to FTS-only when none
//! is provided. Model loading and query embedding happen at the CLI
//! boundary, same pattern as `embed_synced`.

pub mod describe;
pub mod expand;
pub mod hybrid;
pub mod rrf;

pub use expand::ExpansionResult;
pub use hybrid::search;
pub use rrf::fuse;

use crate::storage::crud::Entity;

/// A single search result with provenance.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entity: Entity,
    /// Fused relevance score from RRF (0.0 for graph-expanded results).
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
    /// FTS5 + vector search fused via RRF.
    Hybrid,
    /// FTS5 only (no embedding model available).
    FtsOnly,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::FtsOnly => "fts_only",
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
}

impl Default for SearchParams {
    fn default() -> Self {
        Self {
            entity_type: None,
            status: None,
            limit: 20,
            expand: true,
            max_hops: 2,
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
