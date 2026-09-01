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
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchConfig {
    pub fts_weight: f64,
    pub vec_weight: f64,
    pub rrf_k: u32,
    pub max_results: u32,
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
    /// Default token budget for context packs.
    pub default_token_budget: usize,
    /// Number of recent rules to include in cold_start mode.
    pub cold_start_rules: usize,
    /// Number of recent observations to include in cold_start mode.
    pub cold_start_observations: usize,
    /// Max search results in task mode before expansion.
    pub task_max_results: u32,
    /// Graph expansion hops in task mode.
    pub task_max_hops: usize,
    /// Max search results in escalation mode before expansion.
    pub escalation_max_results: u32,
    /// Graph expansion hops in escalation mode.
    pub escalation_max_hops: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            default_token_budget: 4096,
            cold_start_rules: 5,
            cold_start_observations: 5,
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
            },
            search: SearchConfig {
                fts_weight: 0.4,
                vec_weight: 0.6,
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
}
