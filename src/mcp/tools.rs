//! MCP tool router — dispatches tool calls to implementation modules.
//!
//! Tool implementations are grouped by category:
//! - `tools_write` — record_observation, create_rule, create_knowledge, update_knowledge
//! - `tools_query` — query_observations, query_rules, query_knowledge, list_entities
//! - `tools_search` — search, get_context
//! - `tools_system` — get_status, consolidate
//!
//! Parameter structs live in `params.rs`; shared helpers in `helpers.rs`.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult, tool,
    tool_router,
};

use crate::mcp::params::*;
use crate::mcp::server::CogzServer;
use crate::mcp::{tools_query, tools_search, tools_system, tools_write};

#[tool_router(vis = "pub")]
impl CogzServer {
    #[tool(
        name = "record_observation",
        description = "Record an observation about the codebase. Observations are raw, unvalidated experience — bugs found, decisions made, patterns noticed. They persist across sessions and can be promoted to rules through consolidation."
    )]
    async fn record_observation(
        &self,
        params: Parameters<RecordObservationParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_write::record_observation(self, params).await
    }

    #[tool(
        name = "create_rule",
        description = "Create a rule — a validated directive the agent should follow. Rules are git-tracked and shared. Use for coding standards, design decisions, and confirmed patterns."
    )]
    async fn create_rule(
        &self,
        params: Parameters<CreateRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_write::create_rule(self, params).await
    }

    #[tool(
        name = "create_knowledge",
        description = "Create a knowledge entry — structured documentation about the codebase. Knowledge is human-readable, git-tracked, and meant to be read by both humans and agents."
    )]
    async fn create_knowledge(
        &self,
        params: Parameters<CreateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_write::create_knowledge(self, params).await
    }

    #[tool(
        name = "update_knowledge",
        description = "Update an existing knowledge entry's content. Knowledge is the only entity type that allows in-place content edits — observations and rules are append-only."
    )]
    async fn update_knowledge(
        &self,
        params: Parameters<UpdateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_write::update_knowledge(self, params).await
    }

    #[tool(
        name = "query_observations",
        description = "Query observations with optional filters. Returns matching observations sorted by recency."
    )]
    async fn query_observations(
        &self,
        params: Parameters<QueryObservationsParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_query::query_observations(self, params).await
    }

    #[tool(
        name = "query_rules",
        description = "Query rules with optional filters. Returns matching rules sorted by confidence then recency."
    )]
    async fn query_rules(
        &self,
        params: Parameters<QueryRulesParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_query::query_rules(self, params).await
    }

    #[tool(
        name = "query_knowledge",
        description = "Query knowledge entries with optional filters."
    )]
    async fn query_knowledge(
        &self,
        params: Parameters<QueryKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_query::query_knowledge(self, params).await
    }

    #[tool(
        name = "search",
        description = "Search across all entities using hybrid FTS5 + vector search with RRF fusion. Results include graph expansion — related entities found by following edges."
    )]
    async fn search(
        &self,
        params: Parameters<SearchToolParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_search::search(self, params).await
    }

    #[tool(
        name = "get_context",
        description = "Assemble a context pack — a coherent, scoped, ranked collection of information for the current task. This is the primary output of CogZ."
    )]
    async fn get_context(
        &self,
        params: Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_search::get_context(self, params).await
    }

    #[tool(
        name = "get_status",
        description = "Get CogZ system status: database stats, model availability, entity counts by type, stale entity count."
    )]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        tools_system::get_status(self).await
    }

    #[tool(
        name = "list_entities",
        description = "List all entities of a given type. Returns IDs and titles only — use query tools for full content."
    )]
    async fn list_entities(
        &self,
        params: Parameters<ListEntitiesParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_query::list_entities(self, params).await
    }

    #[tool(
        name = "consolidate",
        description = "Trigger background consolidation: promote supported observations to rules, merge confirmed duplicates. Dedup and contradiction detection happen automatically on every insert; this tool runs the deferred phases."
    )]
    async fn consolidate(
        &self,
        params: Parameters<ConsolidateParams>,
    ) -> Result<CallToolResult, McpError> {
        tools_system::consolidate(self, params).await
    }
}
