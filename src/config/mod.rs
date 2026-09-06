//! Config loading, validation, and default generation.

mod settings;

pub use settings::{
    Config, ConsolidationConfig, ContextConfig, EmbeddingConfig, IndexConfig, ProjectConfig,
    RetentionConfig, SearchConfig, StorageConfig,
};

use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Read(#[from] std::io::Error),
    #[error("failed to parse config TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config validation failed: {0}")]
    Validation(String),
}

/// Load config from a `.cogz/config.toml` file.
///
/// If the file does not exist, returns an error — `cogz init` must be
/// run first. The caller decides whether that's fatal or a prompt to
/// initialize.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let contents = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&contents)?;
    config.validate()?;
    Ok(config)
}

/// Generate the default config TOML string for a given project name.
///
/// Used by `cogz init` to write the initial `config.toml`.
pub fn default_toml(project_name: &str) -> String {
    let config = Config::default_for(project_name);
    toml::to_string(&config).expect("default config serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_roundtrips() {
        let toml_str = default_toml("test-project");
        let config: Config = toml::from_str(&toml_str).unwrap();
        config.validate().unwrap();
        assert_eq!(config.project.name, "test-project");
    }

    #[test]
    fn default_config_values() {
        let config = Config::default_for("my-repo");
        assert_eq!(config.project.name, "my-repo");
        assert_eq!(config.storage.db_path, ".cogz/cogz.db");
        assert_eq!(config.embedding.dimension, 768);
        assert_eq!(config.search.rrf_k, 60);
        assert_eq!(config.consolidation.dedup_threshold, 0.85);
        assert_eq!(config.retention.observation_prune_after_days, 90);
    }

    #[test]
    fn validation_rejects_empty_project_name() {
        let mut config = Config::default_for("test");
        config.project.name = String::new();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("project name"));
    }

    #[test]
    fn validation_rejects_negative_rrf_k() {
        let mut config = Config::default_for("test");
        config.search.rrf_k = 0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("rrf_k"));
    }

    #[test]
    fn validation_rejects_dimension_mismatch() {
        let mut config = Config::default_for("test");
        config.embedding.dimension = 0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("dimension"));
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, default_toml("file-test")).unwrap();

        let config = load(&path).unwrap();
        assert_eq!(config.project.name, "file-test");
    }

    #[test]
    fn load_missing_file_errors() {
        let path = std::path::Path::new("/nonexistent/config.toml");
        assert!(load(path).is_err());
    }
}
