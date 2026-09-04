//! MCP tool parameter structs.
//!
//! Each struct derives `serde::Deserialize` and `schemars::JsonSchema`
//! for automatic JSON Schema generation by rmcp's `#[tool]` macro.
//!
//! `repo` is the first field on every struct — the absolute path to
//! the project root containing `.cogz/`. It is required. The server
//! has no fallbacks; every tool call must specify which repo it
//! targets.

use rmcp::schemars;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecordObservationParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
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
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryObservationsParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub references: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateRuleParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryRulesParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub references: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateKnowledgeParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    pub title: String,
    pub content: String,
    pub category: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateKnowledgeParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
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
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryKnowledgeParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchToolParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
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
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetContextParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub include_stale: Option<bool>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetStatusParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListEntitiesParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    pub entity_type: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConsolidateParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
    /// If true, report what would be consolidated without making changes.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CaptureEventParams {
    /// Absolute path to the project root containing `.cogz/`.
    pub repo: String,
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
}
