//! MCP tool router — tool handler methods on `CogzServer`.
//! Parameter structs live in `params.rs`; helpers in `helpers.rs`.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::*, tool, tool_router,
};
use serde_json::json;

use crate::context::{AssembleParams, ContextMode, assemble_context};
use crate::files::frontmatter::FmValue;
use crate::files::{EntityFile, FileEntityType};
use crate::mcp::helpers::{
    auto_title, context_response, create_entity_file, embed_query_for_search, mcp_internal_error,
    mcp_invalid_parameter, query_response, search_response, tool_success, update_knowledge_file,
    write_and_sync,
};
use crate::mcp::params::*;
use crate::mcp::server::CogzServer;
use crate::search::{SearchParams, search as search_entities};
use crate::storage::{crud, events, query};

#[tool_router(vis = "pub")]
impl CogzServer {
    #[tool(
        name = "record_observation",
        description = "Record an observation about the codebase. Observations are raw, unvalidated experience — bugs found, decisions made, patterns noticed. They persist across sessions and can be promoted to rules through consolidation."
    )]
    async fn record_observation(
        &self,
        Parameters(params): Parameters<RecordObservationParams>,
    ) -> Result<CallToolResult, McpError> {
        let title = params.title.unwrap_or_else(|| auto_title(&params.content));
        let storage = self.storage.clone();
        let config = self.config.clone();
        let cogz_dir = self.cogz_dir.clone();

        let result = tokio::task::spawn_blocking(move || {
            let mut entity = EntityFile::new(&title, FileEntityType::Observation, &params.content);
            if let Some(refs) = params.references.as_deref() {
                entity.references = refs.to_vec();
            }
            if let Some(src) = params.source.as_deref() {
                entity
                    .frontmatter
                    .insert("source", FmValue::String(src.to_string()));
            }
            create_entity_file(&storage, &config, &cogz_dir, &entity)
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

        Ok(tool_success(json!(result)))
    }

    #[tool(
        name = "create_rule",
        description = "Create a rule — a validated directive the agent should follow. Rules are git-tracked and shared. Use for coding standards, design decisions, and confirmed patterns."
    )]
    async fn create_rule(
        &self,
        Parameters(params): Parameters<CreateRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        let title = params.title.unwrap_or_else(|| auto_title(&params.content));
        let storage = self.storage.clone();
        let config = self.config.clone();
        let cogz_dir = self.cogz_dir.clone();

        let result = tokio::task::spawn_blocking(move || {
            let mut entity = EntityFile::new(&title, FileEntityType::Rule, &params.content);
            if let Some(refs) = params.references.as_deref() {
                entity.references = refs.to_vec();
            }
            if let Some(conf) = params.confidence {
                entity
                    .frontmatter
                    .insert("confidence", FmValue::Float(conf));
            }
            create_entity_file(&storage, &config, &cogz_dir, &entity)
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

        Ok(tool_success(json!(result)))
    }

    #[tool(
        name = "create_knowledge",
        description = "Create a knowledge entry — structured documentation about the codebase. Knowledge is human-readable, git-tracked, and meant to be read by both humans and agents."
    )]
    async fn create_knowledge(
        &self,
        Parameters(params): Parameters<CreateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let config = self.config.clone();
        let cogz_dir = self.cogz_dir.clone();

        let result = tokio::task::spawn_blocking(move || {
            let mut entity =
                EntityFile::new(&params.title, FileEntityType::Knowledge, &params.content);
            entity
                .frontmatter
                .insert("category", FmValue::String(params.category.clone()));
            if let Some(tags) = &params.tags {
                entity
                    .frontmatter
                    .insert("tags", FmValue::Array(tags.clone()));
            }
            if let Some(refs) = &params.references {
                entity.references = refs.clone();
            }
            write_and_sync(&storage, &config, &cogz_dir, &entity, "knowledge")
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

        Ok(tool_success(json!(result)))
    }

    #[tool(
        name = "update_knowledge",
        description = "Update an existing knowledge entry's content. Knowledge is the only entity type that allows in-place content edits — observations and rules are append-only."
    )]
    async fn update_knowledge(
        &self,
        Parameters(params): Parameters<UpdateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let cogz_dir = self.cogz_dir.clone();

        let result = tokio::task::spawn_blocking(move || {
            update_knowledge_file(&storage, &cogz_dir, &params)
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;

        Ok(tool_success(json!(result)))
    }

    #[tool(
        name = "query_observations",
        description = "Query observations with optional filters. Returns matching observations sorted by recency."
    )]
    async fn query_observations(
        &self,
        Parameters(params): Parameters<QueryObservationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let entities = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            query::get_entities_by_type(
                &conn,
                "observation",
                params.status.as_deref(),
                params.limit.unwrap_or(20),
            )
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        Ok(tool_success(json!(query_response(
            entities,
            "observations"
        ))))
    }

    #[tool(
        name = "query_rules",
        description = "Query rules with optional filters. Returns matching rules sorted by confidence then recency."
    )]
    async fn query_rules(
        &self,
        Parameters(params): Parameters<QueryRulesParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let entities = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            query::get_entities_by_type(
                &conn,
                "rule",
                params.status.as_deref(),
                params.limit.unwrap_or(20),
            )
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        Ok(tool_success(json!(query_response(entities, "rules"))))
    }

    #[tool(
        name = "query_knowledge",
        description = "Query knowledge entries with optional filters."
    )]
    async fn query_knowledge(
        &self,
        Parameters(params): Parameters<QueryKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let entities = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            let mut entities = query::get_entities_by_type(
                &conn,
                "knowledge",
                params.status.as_deref(),
                params.limit.unwrap_or(20),
            )?;
            if let Some(category) = &params.category {
                entities.retain(|e| {
                    e.properties
                        .get("category")
                        .and_then(|v| v.as_str())
                        .map(|c| c == category)
                        .unwrap_or(false)
                });
            }
            if let Some(tags) = &params.tags {
                entities.retain(|e| {
                    if let Some(entity_tags) = e.properties.get("tags").and_then(|v| v.as_array()) {
                        tags.iter().any(|t| entity_tags.iter().any(|et| et == t))
                    } else {
                        false
                    }
                });
            }
            Ok::<_, crate::storage::StorageError>(entities)
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        Ok(tool_success(json!(query_response(entities, "knowledge"))))
    }

    #[tool(
        name = "search",
        description = "Search across all entities using hybrid FTS5 + vector search with RRF fusion. Results include graph expansion — related entities found by following edges."
    )]
    async fn search(
        &self,
        Parameters(params): Parameters<SearchToolParams>,
    ) -> Result<CallToolResult, McpError> {
        let query_embedding = embed_query_for_search(&self.config, &params.query);
        let storage = self.storage.clone();
        let search_config = self.config.search.clone();
        let default_limit = self.config.search.max_results;

        let results = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            let expand = params.expand.unwrap_or(true);
            let search_params = SearchParams {
                entity_type: params.entity_type,
                status: params.status,
                limit: params.limit.unwrap_or(default_limit),
                expand,
                max_hops: if expand { 2 } else { 0 },
            };
            search_entities(
                &conn,
                &params.query,
                query_embedding.as_deref(),
                &search_params,
                &search_config,
            )
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("search", &e.to_string()))?;

        Ok(tool_success(json!(search_response(results))))
    }

    #[tool(
        name = "get_context",
        description = "Assemble a context pack — a coherent, scoped, ranked collection of information for the current task. This is the primary output of CogZ."
    )]
    async fn get_context(
        &self,
        Parameters(params): Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let mode_str = params.mode.as_deref().unwrap_or("task");
        let mode = ContextMode::parse(mode_str).ok_or_else(|| {
            mcp_invalid_parameter(&format!(
                "invalid mode '{}': expected cold_start, task, or escalation",
                mode_str
            ))
        })?;

        if mode.requires_query() && params.query.is_none() {
            return Err(mcp_invalid_parameter(&format!(
                "query is required for {} mode",
                mode
            )));
        }

        let query_str = params.query.clone();
        let query_embedding = if let Some(ref q) = params.query {
            embed_query_for_search(&self.config, q)
        } else {
            None
        };
        let storage = self.storage.clone();
        let config = self.config.clone();

        let pack = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            let assemble_params = AssembleParams {
                mode,
                query: query_str.as_deref(),
                query_embedding: query_embedding.as_deref(),
                max_tokens: params.max_tokens,
                include_stale: params.include_stale.unwrap_or(false),
            };
            assemble_context(&conn, &assemble_params, &config)
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("context", &e.to_string()))?;

        Ok(tool_success(json!(context_response(pack))))
    }

    #[tool(
        name = "get_status",
        description = "Get CogZ system status: database stats, model availability, entity counts by type, stale entity count."
    )]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let status = tokio::task::spawn_blocking(move || {
            let db_path = {
                let conn = storage.conn();
                let path: Option<String> = conn
                    .query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
                    .ok();
                path
            };
            let db_size = match db_path {
                Some(ref p) if !p.is_empty() => std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                _ => 0,
            };

            let conn = storage.conn();
            let counts = query::entity_counts_by_type(&conn)?;
            let total = crud::count_all(&conn)?;
            let stale = query::count_stale(&conn)?;
            let edges = crate::storage::edges::count_edges(&conn)?;
            let events_count = events::count_events(&conn)?;
            let schema_version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

            Ok::<_, crate::storage::StorageError>(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "schema_version": schema_version,
                "db_size_bytes": db_size,
                "entities": counts.into_iter().collect::<std::collections::HashMap<_, _>>(),
                "total_entities": total,
                "stale_count": stale,
                "edges": edges,
                "events": events_count,
            }))
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("status", &e.to_string()))?;

        Ok(tool_success(status))
    }

    #[tool(
        name = "list_entities",
        description = "List all entities of a given type. Returns IDs and titles only — use query tools for full content."
    )]
    async fn list_entities(
        &self,
        Parameters(params): Parameters<ListEntitiesParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let entity_type = params.entity_type.clone();
        let entities = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            query::get_entities_by_type(&conn, &params.entity_type, params.status.as_deref(), 1000)
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        let items: Vec<_> = entities
            .iter()
            .map(|e| json!({ "id": e.id, "title": e.title }))
            .collect();

        Ok(tool_success(json!({
            "entity_type": entity_type,
            "entities": items,
            "count": items.len(),
        })))
    }
}
