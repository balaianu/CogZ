//! MCP tool parameter structs.
//!
//! Each struct derives `serde::Deserialize` and `schemars::JsonSchema`
//! for automatic JSON Schema generation by rmcp's `#[tool]` macro.
//!
//! All tools accept an optional `repo` parameter — the absolute path
//! to the project root containing `.cogz/`. If omitted, the server
//! falls back to its default repo (set via `--repo` at startup) or cwd.

use rmcp::schemars;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecordObservationParams {
    pub content: String,
    /// Short title. Auto-generated from content if omitted.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    /// UUIDs of observations that this observation supports. Creates
    /// `supports` edges for promotion consolidation.
    #[serde(default)]
    pub supporting_ids: Option<Vec<String>>,
    /// Who or what produced this observation. Default: "agent".
    #[serde(default)]
    pub source: Option<String>,
    /// Absolute path to the project root with `.cogz/`. If omitted,
    /// uses the server default or cwd.
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryObservationsParams {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub references: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateRuleParams {
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryRulesParams {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub references: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateKnowledgeParams {
    pub title: String,
    pub content: String,
    pub category: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateKnowledgeParams {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryKnowledgeParams {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchToolParams {
    pub query: String,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub expand: Option<bool>,
    /// Use the code model (CodeRankEmbed) for query embedding. Applies
    /// the CodeRankEmbed query prefix for code-focused search.
    #[serde(default)]
    pub code_search: Option<bool>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetContextParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub include_stale: Option<bool>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetStatusParams {
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListEntitiesParams {
    pub entity_type: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConsolidateParams {
    /// If true, report what would be consolidated without making changes.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CaptureEventParams {
    /// Event type: session_start, prompt_submit, pre_tool_use, post_tool_use, file_save, session_end.
    pub event_type: String,
    /// Prompt text (for prompt_submit).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Tool name (for pre_tool_use, post_tool_use).
    #[serde(default)]
    pub tool_name: Option<String>,
    /// Tool result summary (for post_tool_use).
    #[serde(default)]
    pub tool_result: Option<String>,
    /// Saved file path, relative to repo root (for file_save).
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
}
