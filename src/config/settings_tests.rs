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
fn allow_and_deny_default_to_empty() {
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
    assert!(config.index.deny.is_empty());
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
task_max_results = 25
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

#[test]
fn validation_rejects_absolute_db_path() {
    let mut config = Config::default_for("test");
    config.storage.db_path = "/etc/passwd".to_string();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("relative"));
}

#[test]
fn validation_rejects_parent_dir_in_db_path() {
    let mut config = Config::default_for("test");
    config.storage.db_path = "../../etc/cogz.db".to_string();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("parent"));
}
