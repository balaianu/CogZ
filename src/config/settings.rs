//! Typed config structs matching `.cogz/config.toml`.
//!
//! Schema defined in `docs/configuration.md`.

use serde::{Deserialize, Serialize};

/// Top-level config. Loaded from `.cogz/config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub project: ProjectConfig,
    pub storage: StorageConfig,
    pub embedding: EmbeddingConfig,
    pub search: SearchConfig,
    pub consolidation: ConsolidationConfig,
    #[serde(default)]
    pub index: IndexConfig,
    pub retention: RetentionConfig,
    #[serde(default)]
    pub context: ContextConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Autodetected on init, saved, versioned. Stable identifier that
    /// survives folder renames. Metadata only — not a query filter.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Per-repo database path, relative to repo root.
    pub db_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub code_model: String,
    pub knowledge_model: String,
    pub dimension: usize,
    /// NLI model ID for contradiction detection (Phase 9).
    /// Empty string = use default (`nli-deberta-v3-xsmall`).
    #[serde(default)]
    pub nli_model: String,
    /// Auto-download models from HuggingFace on first use (default: true).
    #[serde(default = "default_true")]
    pub auto_download: bool,
    /// Seconds of idle time before unloading ONNX models from memory.
    /// 0 = never unload (keep resident for process lifetime).
    /// On a 7GB RAM system, unloading idle models frees ~300-500MB.
    /// Default: 300 (5 minutes).
    #[serde(default = "default_model_idle_ttl")]
    pub model_idle_ttl: u64,
    /// Minimum free memory (MB) required to load a model. If available
    /// RAM drops below this, model loading fails gracefully and the
    /// system degrades to FTS-only. 0 = no check.
    /// Default: 512.
    #[serde(default = "default_model_min_free_mb")]
    pub model_min_free_mb: u64,
}

fn default_true() -> bool {
    true
}

fn default_model_idle_ttl() -> u64 {
    300
}

fn default_model_min_free_mb() -> u64 {
    512
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchConfig {
    pub fts_weight: f64,
    pub vec_weight: f64,
    /// Weight for code vector search results in RRF fusion. Used when
    /// a code query embedding is available. Defaults to 0.3.
    #[serde(default = "default_code_vec_weight")]
    pub code_vec_weight: f64,
    pub rrf_k: u32,
    pub max_results: u32,
    /// Floor for each source type's proportion in balanced fusion.
    /// Ensures neither code nor knowledge is completely suppressed
    /// even when the query strongly favors one. 0.2 = each source
    /// gets at least 20% of the RRF weight. Defaults to 0.2.
    #[serde(default = "default_min_source_proportion")]
    pub min_source_proportion: f64,
    /// Enable query-sensitive source balancing. When true, the hybrid
    /// search detects code/knowledge proportions from KNN distance
    /// spread and FTS pool sizes. When false (default), uses a fixed
    /// 0.5/0.5 split — the evaluation showed this outperforms all
    /// balance detection variants on overall retrieval quality.
    /// The balance detection code is kept for future experimentation
    /// with better signals (e.g. trained classifiers, NLI-based
    /// intent detection).
    #[serde(default = "default_source_balance_enabled")]
    pub source_balance_enabled: bool,
}

fn default_code_vec_weight() -> f64 {
    0.3
}

fn default_min_source_proportion() -> f64 {
    0.2
}

fn default_source_balance_enabled() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    pub dedup_threshold: f64,
    pub title_match_threshold: f64,
    pub contradiction_check: bool,
    pub promotion_threshold: u32,
    /// Minimum P(contradiction) to flag a pair as contradicting.
    /// Calibrated against XNLI dev set in CogZ-py. Below this, the
    /// pair may be related-but-not-contradictory.
    #[serde(default = "default_contradiction_threshold")]
    pub contradiction_threshold: f64,
    /// Minimum embedding cosine similarity for a contradiction pair.
    /// Genuine contradictions share the same topic with opposing
    /// claims, so their embeddings should be very similar.
    #[serde(default = "default_contradiction_cosine_threshold")]
    pub contradiction_cosine_threshold: f64,
    /// Maximum text length ratio for a contradiction pair. Texts
    /// differing by more than this ratio are likely different content
    /// types, not a genuine contradiction.
    #[serde(default = "default_contradiction_length_ratio")]
    pub contradiction_length_ratio: f64,
    /// Minimum bidirectional P(entailment) to confirm a duplicate pair.
    /// Both A entails B AND B entails A must score above this. True
    /// duplicates entail mutually; a subset-fact does not.
    #[serde(default = "default_dedup_nli_threshold")]
    pub dedup_nli_threshold: f64,
}

fn default_contradiction_threshold() -> f64 {
    0.70
}
fn default_contradiction_cosine_threshold() -> f64 {
    0.85
}
fn default_contradiction_length_ratio() -> f64 {
    5.0
}
fn default_dedup_nli_threshold() -> f64 {
    0.85
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IndexConfig {
    /// Glob patterns for files to index despite being gitignored.
    /// Patterns are relative to the repo root and use standard glob
    /// syntax (`*`, `**`, `?`, `[abc]`). Allow overrides both gitignore
    /// and deny.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Glob patterns for files to exclude from indexing even if they
    /// are not gitignored. Patterns are relative to the repo root and
    /// use the same glob syntax as `allow`. Use cases: vendored code,
    /// generated files not covered by .gitignore, benchmark files.
    /// Allow patterns take precedence over deny.
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetentionConfig {
    pub observation_prune_after_days: u32,
    pub tombstone_max_count: u32,
}

/// Context assembly configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Default token budget for cold_start context packs.
    pub default_token_budget: usize,
    /// Token budget for task context packs. Larger than cold_start
    /// to accommodate code entities alongside knowledge entries.
    #[serde(default = "default_task_token_budget")]
    pub task_token_budget: usize,
    /// Token budget for escalation context packs. Same as task by
    /// default — escalation widens search depth, not just budget.
    #[serde(default = "default_escalation_token_budget")]
    pub escalation_token_budget: usize,
    /// Number of recent rules to include in cold_start mode.
    pub cold_start_rules: usize,
    /// Max search results in task mode before expansion.
    pub task_max_results: u32,
    /// Graph expansion hops in task mode.
    pub task_max_hops: usize,
    /// Max search results in escalation mode before expansion.
    pub escalation_max_results: u32,
    /// Graph expansion hops in escalation mode.
    pub escalation_max_hops: usize,
}

fn default_task_token_budget() -> usize {
    8192
}

fn default_escalation_token_budget() -> usize {
    8192
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            default_token_budget: 4096,
            task_token_budget: 8192,
            escalation_token_budget: 8192,
            cold_start_rules: 5,
            task_max_results: 25,
            task_max_hops: 2,
            escalation_max_results: 20,
            escalation_max_hops: 3,
        }
    }
}

impl Config {
    /// Create a default config for a given project name.
    pub fn default_for(project_name: &str) -> Self {
        Self {
            project: ProjectConfig {
                name: project_name.to_string(),
            },
            storage: StorageConfig {
                db_path: ".cogz/cogz.db".to_string(),
            },
            embedding: EmbeddingConfig {
                code_model: crate::embed::registry::DEFAULT_CODE_MODEL.to_string(),
                knowledge_model: crate::embed::registry::DEFAULT_KNOWLEDGE_MODEL.to_string(),
                dimension: crate::embed::registry::DEFAULT_DIMENSION,
                nli_model: crate::embed::registry::DEFAULT_NLI_MODEL.to_string(),
                auto_download: true,
                model_idle_ttl: 300,
                model_min_free_mb: 512,
            },
            search: SearchConfig {
                fts_weight: 0.3,
                vec_weight: 0.4,
                code_vec_weight: 0.3,
                rrf_k: 60,
                max_results: 20,
                min_source_proportion: 0.2,
                source_balance_enabled: false,
            },
            consolidation: ConsolidationConfig {
                dedup_threshold: 0.85,
                title_match_threshold: 0.85,
                contradiction_check: true,
                promotion_threshold: 3,
                contradiction_threshold: 0.70,
                contradiction_cosine_threshold: 0.85,
                contradiction_length_ratio: 5.0,
                dedup_nli_threshold: 0.85,
            },
            index: IndexConfig {
                allow: vec![],
                deny: vec![],
            },
            retention: RetentionConfig {
                observation_prune_after_days: 90,
                tombstone_max_count: 1000,
            },
            context: ContextConfig::default(),
        }
    }

    /// Validate config values. Called after loading and after
    /// generating defaults.
    pub fn validate(&self) -> Result<(), super::ConfigError> {
        if self.project.name.trim().is_empty() {
            return Err(super::ConfigError::Validation(
                "project name must not be empty".to_string(),
            ));
        }
        // db_path must be relative, contained beneath .cogz/, and
        // have no parent-component traversal. This prevents the
        // configured path from opening or creating a database outside
        // the repository. Defense-in-depth canonicalization at open
        // time further guards against symlinks.
        let db_path = std::path::Path::new(&self.storage.db_path);
        if db_path.is_absolute() {
            return Err(super::ConfigError::Validation(
                "storage.db_path must be a relative path beneath .cogz/".to_string(),
            ));
        }
        if db_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(super::ConfigError::Validation(
                "storage.db_path must not contain '..' parent components".to_string(),
            ));
        }
        if !self.storage.db_path.starts_with(".cogz/") {
            return Err(super::ConfigError::Validation(
                "storage.db_path must be rooted beneath .cogz/ (e.g. '.cogz/cogz.db')".to_string(),
            ));
        }
        if self.embedding.dimension == 0 {
            return Err(super::ConfigError::Validation(
                "embedding dimension must be greater than 0".to_string(),
            ));
        }
        // Cross-reference configured dimension against the model registry.
        // A mismatch means the vec0 table is created at one dimension while
        // the model produces vectors at another — KNN will fail silently.
        for (model_id, label) in [
            (&self.embedding.code_model, "code_model"),
            (&self.embedding.knowledge_model, "knowledge_model"),
        ] {
            if let Some(entry) = crate::embed::registry::lookup(model_id)
                && entry.dim != self.embedding.dimension
            {
                return Err(super::ConfigError::Validation(format!(
                    "embedding.dimension ({}) does not match {} registry dimension ({}) for model '{}'. \
                     Set dimension to {} or change the model.",
                    self.embedding.dimension, label, entry.dim, model_id, entry.dim
                )));
            }
        }
        if self.search.rrf_k == 0 {
            return Err(super::ConfigError::Validation(
                "search.rrf_k must be greater than 0".to_string(),
            ));
        }
        if self.search.max_results == 0 {
            return Err(super::ConfigError::Validation(
                "search.max_results must be greater than 0".to_string(),
            ));
        }
        if self.consolidation.dedup_threshold < 0.0 || self.consolidation.dedup_threshold > 1.0 {
            return Err(super::ConfigError::Validation(
                "consolidation.dedup_threshold must be between 0.0 and 1.0".to_string(),
            ));
        }
        if self.consolidation.title_match_threshold < 0.0
            || self.consolidation.title_match_threshold > 1.0
        {
            return Err(super::ConfigError::Validation(
                "consolidation.title_match_threshold must be between 0.0 and 1.0".to_string(),
            ));
        }
        if self.retention.observation_prune_after_days == 0 {
            return Err(super::ConfigError::Validation(
                "retention.observation_prune_after_days must be greater than 0".to_string(),
            ));
        }
        if self.context.default_token_budget == 0 {
            return Err(super::ConfigError::Validation(
                "context.default_token_budget must be greater than 0".to_string(),
            ));
        }
        if self.context.task_token_budget == 0 {
            return Err(super::ConfigError::Validation(
                "context.task_token_budget must be greater than 0".to_string(),
            ));
        }
        if self.context.escalation_token_budget == 0 {
            return Err(super::ConfigError::Validation(
                "context.escalation_token_budget must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
