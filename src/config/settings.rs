//! Typed config structs matching `.cogz/config.toml`.
//!
//! Schema defined in `docs/architecture.md` → Configuration section.

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
}

fn default_code_vec_weight() -> f64 {
    0.3
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
    /// Explicit gitignore overrides — paths to index despite being
    /// gitignored.
    #[serde(default)]
    pub allow: Vec<String>,
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
            task_max_results: 10,
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
            },
            consolidation: ConsolidationConfig {
                dedup_threshold: 0.92,
                title_match_threshold: 0.85,
                contradiction_check: true,
                promotion_threshold: 3,
                contradiction_threshold: 0.70,
                contradiction_cosine_threshold: 0.85,
                contradiction_length_ratio: 5.0,
                dedup_nli_threshold: 0.85,
            },
            index: IndexConfig { allow: vec![] },
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
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let config = Config::default_for("roundtrip-test");
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.project.name, "roundtrip-test");
        assert_eq!(parsed, config);
    }

    #[test]
    fn allow_defaults_to_empty() {
        let toml_str = r#"
[project]
name = "test"

[storage]
db_path = ".cogz/cogz.db"

[embedding]
code_model = "test"
knowledge_model = "test"
dimension = 384

[search]
fts_weight = 0.4
vec_weight = 0.6
rrf_k = 60
max_results = 20

[consolidation]
dedup_threshold = 0.92
title_match_threshold = 0.85
contradiction_check = true
promotion_threshold = 3

[retention]
observation_prune_after_days = 90
tombstone_max_count = 1000
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.index.allow.is_empty());
        assert_eq!(config.context, ContextConfig::default());
    }

    #[test]
    fn old_config_without_per_mode_token_budgets_uses_serde_defaults() {
        let toml_str = r#"
[project]
name = "test"

[storage]
db_path = ".cogz/cogz.db"

[embedding]
code_model = "test"
knowledge_model = "test"
dimension = 384

[search]
fts_weight = 0.4
vec_weight = 0.6
rrf_k = 60
max_results = 20

[consolidation]
dedup_threshold = 0.92
title_match_threshold = 0.85
contradiction_check = true
promotion_threshold = 3

[context]
default_token_budget = 4096
cold_start_rules = 5
task_max_results = 10
task_max_hops = 2
escalation_max_results = 20
escalation_max_hops = 3

[retention]
observation_prune_after_days = 90
tombstone_max_count = 1000
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        // Old configs without task/escalation token budget fields
        // must fall back to serde defaults, not fail or panic.
        assert_eq!(config.context.default_token_budget, 4096);
        assert_eq!(config.context.task_token_budget, 8192);
        assert_eq!(config.context.escalation_token_budget, 8192);
        assert!(config.validate().is_ok());
    }
}
