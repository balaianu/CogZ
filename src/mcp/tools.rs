//! MCP tool router — tool handler methods on `CogzServer`.
//! Parameter structs live in `params.rs`; helpers in `helpers.rs`.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::*, tool, tool_router,
};
use serde_json::json;

use crate::context::{AssembleParams, assemble_context};
use crate::files::frontmatter::FmValue;
use crate::files::{EntityFile, FileEntityType};
use crate::mcp::helpers::{
    auto_title, build_status_response, create_entity_file, embed_query_for_search,
    list_entities_response, mcp_internal_error, parse_context_mode, query_by_type_with_refs,
    query_knowledge_with_refs, update_knowledge_file, write_and_sync,
};
use crate::mcp::params::*;
use crate::mcp::responses::{context_response, query_response, search_response, tool_success};
use crate::mcp::server::CogzServer;
use crate::search::{SearchParams, search as search_entities};

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
        let query_model = self.query_model.clone();
        let nli_model = self.nli_model.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut entity = EntityFile::new(&title, FileEntityType::Observation, &params.content);
            if let Some(refs) = params.references.as_deref() {
                entity.references = refs.to_vec();
            }
            let source = params.source.unwrap_or_else(|| "agent".to_string());
            entity.frontmatter.insert("source", FmValue::String(source));
            entity.frontmatter.insert("confidence", FmValue::Float(0.5));
            create_entity_file(
                &storage,
                &config,
                &cogz_dir,
                &entity,
                Some(&query_model),
                Some(&*nli_model),
            )
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
        let query_model = self.query_model.clone();
        let nli_model = self.nli_model.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut entity = EntityFile::new(&title, FileEntityType::Rule, &params.content);
            if let Some(refs) = params.references.as_deref() {
                entity.references = refs.to_vec();
            }
            let confidence = params.confidence.unwrap_or(1.0);
            entity
                .frontmatter
                .insert("confidence", FmValue::Float(confidence));
            create_entity_file(
                &storage,
                &config,
                &cogz_dir,
                &entity,
                Some(&query_model),
                Some(&*nli_model),
            )
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
        let query_model = self.query_model.clone();

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
            write_and_sync(
                &storage,
                &config,
                &cogz_dir,
                &entity,
                "knowledge",
                Some(&query_model),
                None,
            )
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
        let query_model = self.query_model.clone();
        let result = tokio::task::spawn_blocking(move || {
            update_knowledge_file(&storage, &cogz_dir, &params, Some(&query_model))
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
        let result = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            query_by_type_with_refs(
                &conn,
                "observation",
                params.status.as_deref(),
                params.limit.unwrap_or(20),
                params.references.as_deref(),
            )
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        let (entities, refs_map) = result;
        Ok(tool_success(json!(query_response(
            entities,
            "observations",
            &refs_map
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
        let result = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            query_by_type_with_refs(
                &conn,
                "rule",
                params.status.as_deref(),
                params.limit.unwrap_or(20),
                params.references.as_deref(),
            )
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        let (entities, refs_map) = result;
        Ok(tool_success(json!(query_response(
            entities, "rules", &refs_map
        ))))
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
        let result = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            query_knowledge_with_refs(
                &conn,
                params.status.as_deref(),
                params.category.as_deref(),
                params.tags.as_deref(),
                params.limit.unwrap_or(20),
            )
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;

        let (entities, refs_map) = result;
        Ok(tool_success(json!(query_response(
            entities,
            "knowledge",
            &refs_map
        ))))
    }

    #[tool(
        name = "search",
        description = "Search across all entities using hybrid FTS5 + vector search with RRF fusion. Results include graph expansion — related entities found by following edges."
    )]
    async fn search(
        &self,
        Parameters(params): Parameters<SearchToolParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let search_config = self.config.search.clone();
        let default_limit = self.config.search.max_results;
        let task_max_hops = self.config.context.task_max_hops;
        let query_model = self.query_model.clone();
        let results = tokio::task::spawn_blocking(move || {
            let query_embedding = embed_query_for_search(&query_model, &params.query);
            let conn = storage.conn();
            let expand = params.expand.unwrap_or(true);
            let search_params = SearchParams {
                entity_type: params.entity_type,
                status: params.status,
                limit: params.limit.unwrap_or(default_limit),
                expand,
                max_hops: if expand { task_max_hops } else { 0 },
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
        let mode = parse_context_mode(mode_str, params.query.as_deref())?;
        let query_str = params.query.clone();
        let storage = self.storage.clone();
        let config = self.config.clone();
        let query_model = self.query_model.clone();
        let pack = tokio::task::spawn_blocking(move || {
            let query_embedding = params
                .query
                .as_deref()
                .and_then(|q| embed_query_for_search(&query_model, q));
            let conn = storage.conn();
            assemble_context(
                &conn,
                &AssembleParams {
                    mode,
                    query: query_str.as_deref(),
                    query_embedding: query_embedding.as_deref(),
                    max_tokens: params.max_tokens,
                    include_stale: params.include_stale.unwrap_or(false),
                },
                &config,
            )
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
        let config = self.config.clone();
        let status = tokio::task::spawn_blocking(move || build_status_response(&storage, &config))
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
        let status = params.status.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = storage.conn();
            list_entities_response(&conn, &entity_type, status.as_deref())
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))?
        .map_err(|e| mcp_internal_error("query", &e.to_string()))?;
        Ok(tool_success(result))
    }

    #[tool(
        name = "consolidate",
        description = "Trigger background consolidation: promote supported observations to rules, merge confirmed duplicates. Dedup and contradiction detection happen automatically on every insert; this tool runs the deferred phases."
    )]
    async fn consolidate(
        &self,
        Parameters(params): Parameters<ConsolidateParams>,
    ) -> Result<CallToolResult, McpError> {
        let storage = self.storage.clone();
        let config = self.config.clone();
        let cogz_dir = self.cogz_dir.clone();
        let dry_run = params.dry_run;
        let result = tokio::task::spawn_blocking(move || {
            let promoted = crate::consolidate::promote::run_promotion(
                &storage,
                &cogz_dir,
                &config.consolidation,
                dry_run,
            )
            .map_err(|e| mcp_internal_error("consolidate", &e.to_string()))?;

            let merged = crate::consolidate::merge::run_merge(
                &storage,
                &cogz_dir,
                &config.consolidation,
                dry_run,
            )
            .map_err(|e| mcp_internal_error("consolidate", &e.to_string()))?;
            Ok::<_, McpError>(json!({
                "promoted": promoted,
                "merged": merged,
                "promoted_count": promoted.len(),
                "merged_count": merged.len(),
                "dry_run": dry_run,
            }))
        })
        .await
        .map_err(|e| mcp_internal_error("spawn_blocking", &e.to_string()))??;
        Ok(tool_success(result))
    }
}
