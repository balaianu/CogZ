//! Integration tests for the MCP server tools.
//!
//! Tests use a tokio::io::duplex transport pair to connect a real
//! rmcp client to the CogzServer. This exercises the full MCP
//! protocol path: tool routing, parameter deserialization, handler
//! execution, and response serialization.

use std::sync::Arc;

use cogz::config::Config;
use cogz::mcp::CogzServer;
use cogz::storage::Storage;
use rmcp::model::CallToolRequestParams;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::json;

fn setup() -> (CogzServer, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cogz_dir = dir.path().join(".cogz");
    std::fs::create_dir_all(&cogz_dir).unwrap();
    std::fs::create_dir_all(cogz_dir.join("observations")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("rules")).unwrap();
    std::fs::create_dir_all(cogz_dir.join("knowledge")).unwrap();

    let storage = Arc::new(Storage::open_memory().unwrap());
    let config = Config::default_for("test-project");
    let server = CogzServer::new(storage, config, cogz_dir);
    (server, dir)
}

struct DummyClient;
impl ClientHandler for DummyClient {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::default()
    }
}

/// Spawn the CogzServer on a duplex transport and return a connected client service.
async fn spawn_server(
    server: CogzServer,
) -> rmcp::service::RunningService<rmcp::RoleClient, DummyClient> {
    let (server_transport, client_transport) = tokio::io::duplex(4096);
    let _server_handle = tokio::spawn(async move {
        match server.serve(server_transport).await {
            Ok(service) => {
                let _ = service.waiting().await;
            }
            Err(e) => {
                eprintln!("Server serve error: {:?}", e);
            }
        }
    });
    let client = DummyClient.serve(client_transport).await.unwrap();
    // Give the server a moment to initialize
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    client
}

fn call_tool_args(args: serde_json::Value) -> Option<rmcp::model::JsonObject> {
    args.as_object().map(|m| m.clone().into_iter().collect())
}

fn parse_result(result: rmcp::model::CallToolResult) -> serde_json::Value {
    assert!(
        !result.is_error.unwrap_or(false),
        "Tool returned error: {:?}",
        result.content
    );
    let text = result
        .content
        .first()
        .expect("result has content")
        .as_text()
        .expect("content is text")
        .text
        .as_str();
    serde_json::from_str(text).expect("result is valid JSON")
}

// ── get_status ────────────────────────────────────────────────────

#[tokio::test]
async fn get_status_returns_db_stats() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(CallToolRequestParams::new("get_status"))
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert!(value["schema_version"].as_u64().is_some());
    assert_eq!(value["total_entities"], 0);
    assert_eq!(value["stale_count"], 0);
}

// ── record_observation ────────────────────────────────────────────

#[tokio::test]
async fn record_observation_creates_file_and_db_entry() {
    let (server, dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Found a bug in the search ranking logic",
                    "title": "Search ranking bug",
                    "source": "test",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert!(value["id"].as_str().is_some());
    assert!(
        value["file_path"]
            .as_str()
            .unwrap()
            .contains("observations")
    );
    assert_eq!(value["status"], "active");

    // Verify file was written
    let cogz_dir = dir.path().join(".cogz");
    let obs_dir = cogz_dir.join("observations");
    let files: Vec<_> = std::fs::read_dir(&obs_dir).unwrap().collect();
    assert!(!files.is_empty(), "at least one observation file exists");
}

// ── create_rule ───────────────────────────────────────────────────

#[tokio::test]
async fn create_rule_creates_file_and_db_entry() {
    let (server, dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("create_rule").with_arguments(
                call_tool_args(json!({
                    "content": "Always use parameterized SQL queries",
                    "title": "Use parameterized queries",
                    "confidence": 0.9,
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert!(value["id"].as_str().is_some());
    assert!(value["file_path"].as_str().unwrap().contains("rules"));

    let rules_dir = dir.path().join(".cogz").join("rules");
    let files: Vec<_> = std::fs::read_dir(&rules_dir).unwrap().collect();
    assert!(!files.is_empty(), "rule file exists");
}

// ── create_knowledge ──────────────────────────────────────────────

#[tokio::test]
async fn create_knowledge_creates_file_and_db_entry() {
    let (server, dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("create_knowledge").with_arguments(
                call_tool_args(json!({
                    "title": "Search Architecture",
                    "content": "The search pipeline uses FTS5 and optional vector search.",
                    "category": "architecture",
                    "tags": ["search", "architecture"],
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert!(value["id"].as_str().is_some());
    assert!(value["file_path"].as_str().unwrap().contains("knowledge"));
    assert!(
        value["file_path"]
            .as_str()
            .unwrap()
            .contains("architecture")
    );

    let knowledge_dir = dir
        .path()
        .join(".cogz")
        .join("knowledge")
        .join("architecture");
    let files: Vec<_> = std::fs::read_dir(&knowledge_dir).unwrap().collect();
    assert!(!files.is_empty(), "knowledge file exists in category dir");
}

// ── query_observations ────────────────────────────────────────────

#[tokio::test]
async fn query_observations_returns_results() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    // Create an observation first
    let _ = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Test observation content",
                    "title": "Test observation",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Query observations
    let result = client
        .call_tool(CallToolRequestParams::new("query_observations"))
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["count"], 1);
    assert!(value["observations"].is_array());
    assert_eq!(value["observations"][0]["title"], "Test observation");
}

// ── query_rules ───────────────────────────────────────────────────

#[tokio::test]
async fn query_rules_returns_results() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let _ = client
        .call_tool(
            CallToolRequestParams::new("create_rule").with_arguments(
                call_tool_args(json!({
                    "content": "Always batch DB queries",
                    "title": "Batch DB queries",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    let result = client
        .call_tool(CallToolRequestParams::new("query_rules"))
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["count"], 1);
    assert_eq!(value["rules"][0]["title"], "Batch DB queries");
}

// ── query_knowledge ───────────────────────────────────────────────

#[tokio::test]
async fn query_knowledge_filters_by_category() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    for (title, category) in [("Search Arch", "architecture"), ("DB Design", "decisions")] {
        let _ = client
            .call_tool(
                CallToolRequestParams::new("create_knowledge").with_arguments(
                    call_tool_args(json!({
                        "title": title,
                        "content": "content",
                        "category": category,
                    }))
                    .unwrap(),
                ),
            )
            .await
            .unwrap();
    }

    let result = client
        .call_tool(
            CallToolRequestParams::new("query_knowledge")
                .with_arguments(call_tool_args(json!({"category": "architecture"})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["count"], 1);
    assert_eq!(value["knowledge"][0]["title"], "Search Arch");
}

// ── search ────────────────────────────────────────────────────────

#[tokio::test]
async fn search_returns_fts_results() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    for (title, content) in [
        ("FTS5 ranking", "Full text search ranking with BM25"),
        ("Vector search", "Embedding-based semantic search"),
    ] {
        let _ = client
            .call_tool(
                CallToolRequestParams::new("create_knowledge").with_arguments(
                    call_tool_args(json!({
                        "title": title,
                        "content": content,
                        "category": "search",
                    }))
                    .unwrap(),
                ),
            )
            .await
            .unwrap();
    }

    let result = client
        .call_tool(
            CallToolRequestParams::new("search")
                .with_arguments(call_tool_args(json!({"query": "ranking"})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert!(value["count"].as_u64().unwrap() > 0);
    assert!(value["results"].is_array());
}

// ── get_context ───────────────────────────────────────────────────

#[tokio::test]
async fn get_context_cold_start() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let _ = client
        .call_tool(
            CallToolRequestParams::new("create_rule").with_arguments(
                call_tool_args(json!({
                    "content": "Always use typed errors",
                    "title": "Typed errors rule",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_context")
                .with_arguments(call_tool_args(json!({"mode": "cold_start"})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["mode"], "cold_start");
    assert!(value["sections"].is_array());
}

#[tokio::test]
async fn get_context_task_requires_query() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_context")
                .with_arguments(call_tool_args(json!({"mode": "task"})).unwrap()),
        )
        .await;
    assert!(result.is_err(), "task mode without query should error");
}

#[tokio::test]
async fn get_context_invalid_mode_errors() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_context")
                .with_arguments(call_tool_args(json!({"mode": "invalid_mode"})).unwrap()),
        )
        .await;
    assert!(result.is_err(), "invalid mode should error");
}

// ── list_entities ─────────────────────────────────────────────────

#[tokio::test]
async fn list_entities_returns_ids_and_titles() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let _ = client
        .call_tool(
            CallToolRequestParams::new("create_rule").with_arguments(
                call_tool_args(json!({
                    "content": "Test rule",
                    "title": "Test rule title",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    let result = client
        .call_tool(
            CallToolRequestParams::new("list_entities")
                .with_arguments(call_tool_args(json!({"entity_type": "rule"})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["entity_type"], "rule");
    assert_eq!(value["count"], 1);
    assert_eq!(value["entities"][0]["title"], "Test rule title");
}

// ── update_knowledge ──────────────────────────────────────────────

#[tokio::test]
async fn update_knowledge_edits_content() {
    let (server, dir) = setup();
    let storage = server.storage.clone();
    let client = spawn_server(server).await;

    // Create knowledge
    let create_result = client
        .call_tool(
            CallToolRequestParams::new("create_knowledge").with_arguments(
                call_tool_args(json!({
                    "title": "Original title",
                    "content": "Original content",
                    "category": "test",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let create_value = parse_result(create_result);
    let entity_id = create_value["id"].as_str().unwrap();

    // Update it
    let result = client
        .call_tool(
            CallToolRequestParams::new("update_knowledge").with_arguments(
                call_tool_args(json!({
                    "id": entity_id,
                    "content": "Updated content",
                    "title": "Updated title",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert_eq!(value["id"], entity_id);
    let updated = value["updated_fields"].as_array().unwrap();
    assert!(updated.contains(&json!("content")));
    assert!(updated.contains(&json!("title")));

    // Verify DB has updated content
    let conn = storage.conn();
    let entity = cogz::storage::crud::get_entity(&conn, entity_id).unwrap();
    assert_eq!(entity.content, "Updated content");
    assert_eq!(entity.title, Some("Updated title".to_string()));

    // Verify file on disk has updated content
    let cogz_dir = dir.path().join(".cogz");
    let kn_dir = cogz_dir.join("knowledge").join("test");
    let files: Vec<_> = std::fs::read_dir(&kn_dir).unwrap().collect();
    assert_eq!(files.len(), 1);
    let content = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
    assert!(content.contains("Updated content"));
}

#[tokio::test]
async fn update_knowledge_nonexistent_id_errors() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("update_knowledge").with_arguments(
                call_tool_args(json!({
                    "id": "nonexistent-uuid",
                    "content": "content",
                }))
                .unwrap(),
            ),
        )
        .await;
    assert!(result.is_err(), "nonexistent ID should error");
}

// ── dedup ─────────────────────────────────────────────────────────

#[tokio::test]
async fn record_observation_duplicate_title_warns() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    // First observation
    let _ = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "First observation",
                    "title": "Duplicate title",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Second observation with same title
    let result = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Second observation",
                    "title": "Duplicate title",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);

    assert!(
        value["duplicate_warning"].is_object(),
        "duplicate warning should be present"
    );
    assert_eq!(value["duplicate_warning"]["title_match"], "exact");
}

// ── rebuildability ────────────────────────────────────────────────

#[tokio::test]
async fn db_rebuildable_from_files_after_mcp_writes() {
    let (server, dir) = setup();
    let client = spawn_server(server).await;

    // Write entities via MCP tools
    for (tool, args) in [
        (
            "record_observation",
            json!({"content": "obs content", "title": "obs title"}),
        ),
        (
            "create_rule",
            json!({"content": "rule content", "title": "rule title"}),
        ),
        (
            "create_knowledge",
            json!({"title": "kn title", "content": "kn content", "category": "test"}),
        ),
    ] {
        let _ = client
            .call_tool(
                CallToolRequestParams::new(tool).with_arguments(call_tool_args(args).unwrap()),
            )
            .await
            .unwrap();
    }

    // The server holds an in-memory DB; verify files exist on disk
    let cogz_dir = dir.path().join(".cogz");
    let obs_files: Vec<_> = std::fs::read_dir(cogz_dir.join("observations"))
        .unwrap()
        .collect();
    let rule_files: Vec<_> = std::fs::read_dir(cogz_dir.join("rules")).unwrap().collect();
    let kn_files: Vec<_> = std::fs::read_dir(cogz_dir.join("knowledge").join("test"))
        .unwrap()
        .collect();
    assert_eq!(obs_files.len(), 1);
    assert_eq!(rule_files.len(), 1);
    assert_eq!(kn_files.len(), 1);

    // Rebuild DB from files using a fresh storage
    let config = Config::default_for("test-project");
    let db_path = dir.path().join(&config.storage.db_path);
    let new_storage = Storage::open(&db_path).unwrap();
    let sync_result = cogz::files::sync_all(&new_storage, &cogz_dir);
    assert!(sync_result.errors.is_empty(), "sync has no errors");

    let conn = new_storage.conn();
    let count = cogz::storage::crud::count_all(&conn).unwrap();
    assert_eq!(count, 3, "all entities rebuilt from files");
}

// ── query status defaults ─────────────────────────────────────────

#[tokio::test]
async fn query_observations_defaults_to_active_status() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    // Create an observation
    let _ = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Active observation content",
                    "title": "Active obs",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Query with no status filter — should default to active and return it
    let result = client
        .call_tool(
            CallToolRequestParams::new("query_observations")
                .with_arguments(call_tool_args(json!({})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    assert_eq!(value["count"], 1, "active observation returned by default");

    // Query with status="all" — should also return it
    let result = client
        .call_tool(
            CallToolRequestParams::new("query_observations")
                .with_arguments(call_tool_args(json!({"status": "all"})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    assert_eq!(
        value["count"], 1,
        "status=all returns all observations including active"
    );
}

// ── query response includes references ────────────────────────────

#[tokio::test]
async fn query_response_includes_references_field() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    // Create a knowledge entry first to get a reference target
    let kn_result = client
        .call_tool(
            CallToolRequestParams::new("create_knowledge").with_arguments(
                call_tool_args(json!({
                    "title": "Architecture knowledge",
                    "content": "The system uses SQLite",
                    "category": "architecture",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let kn_value = parse_result(kn_result);
    let kn_id = kn_value["id"].as_str().unwrap();

    // Create an observation that references the knowledge
    let _ = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Confirmed the architecture",
                    "title": "Architecture confirmed",
                    "references": [kn_id],
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Query observations — should include references field
    let result = client
        .call_tool(
            CallToolRequestParams::new("query_observations")
                .with_arguments(call_tool_args(json!({})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    let obs = &value["observations"][0];
    assert!(
        obs["references"].is_array(),
        "references field is present and is an array"
    );
    let refs = obs["references"].as_array().unwrap();
    assert_eq!(refs.len(), 1, "one reference");
    assert_eq!(refs[0], kn_id, "reference matches the knowledge ID");
}

// ── query with references filter returns correct count ────────────

#[tokio::test]
async fn query_observations_with_references_filter() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    // Create a knowledge entry as the reference target
    let kn_result = client
        .call_tool(
            CallToolRequestParams::new("create_knowledge").with_arguments(
                call_tool_args(json!({
                    "title": "Reference target",
                    "content": "Target knowledge",
                    "category": "test",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let kn_id = parse_result(kn_result)["id"].as_str().unwrap().to_string();

    // Create 3 observations: 2 referencing the knowledge, 1 not
    for i in 0..3 {
        let refs = if i < 2 { json!([kn_id]) } else { json!([]) };
        let _ = client
            .call_tool(
                CallToolRequestParams::new("record_observation").with_arguments(
                    call_tool_args(json!({
                        "content": format!("Observation {i}"),
                        "title": format!("Obs {i}"),
                        "references": refs,
                    }))
                    .unwrap(),
                ),
            )
            .await
            .unwrap();
    }

    // Query with references filter — should return 2, not be limited
    // by post-fetch filtering
    let result = client
        .call_tool(
            CallToolRequestParams::new("query_observations").with_arguments(
                call_tool_args(json!({
                    "references": kn_id,
                    "limit": 20,
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    assert_eq!(
        value["count"], 2,
        "references filter returns exactly the 2 matching observations"
    );
}

// ── query knowledge response includes category and tags ───────────

#[tokio::test]
async fn query_knowledge_response_includes_category_and_tags() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let _ = client
        .call_tool(
            CallToolRequestParams::new("create_knowledge").with_arguments(
                call_tool_args(json!({
                    "title": "Tagged knowledge",
                    "content": "Important info",
                    "category": "testing",
                    "tags": ["rust", "sqlite"],
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    let result = client
        .call_tool(
            CallToolRequestParams::new("query_knowledge")
                .with_arguments(call_tool_args(json!({})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    let kn = &value["knowledge"][0];
    assert_eq!(kn["category"], "testing", "category field present");
    assert!(kn["tags"].is_array(), "tags field present and is an array");
    let tags = kn["tags"].as_array().unwrap();
    assert!(tags.len() == 2, "two tags present");
}

// ── get_status includes models and db_path ────────────────────────

#[tokio::test]
async fn get_status_includes_models_and_db_path() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(CallToolRequestParams::new("get_status"))
        .await
        .unwrap();
    let value = parse_result(result);

    assert!(
        value["models"].is_object(),
        "models field is present and is an object"
    );
    assert!(
        value["models"]["embedding_knowledge"]["available"].is_boolean(),
        "embedding_knowledge.available is a boolean"
    );
    assert!(
        value["models"]["embedding_knowledge"]["name"].is_string(),
        "embedding_knowledge.name is a string"
    );
    assert!(
        value["models"]["embedding_code"]["available"].is_boolean(),
        "embedding_code.available is a boolean"
    );
    assert!(
        value["models"]["nli"]["available"].is_boolean(),
        "nli.available is a boolean"
    );
    assert!(
        value["db_path"].is_string(),
        "db_path field is present and is a string"
    );
}

// ── record_observation defaults source to "agent" ─────────────────

#[tokio::test]
async fn record_observation_defaults_source_to_agent() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Observation without explicit source",
                    "title": "No source obs",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let _ = parse_result(result);

    // Query it back and verify source defaults to "agent"
    let result = client
        .call_tool(
            CallToolRequestParams::new("query_observations")
                .with_arguments(call_tool_args(json!({})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    let obs = &value["observations"][0];
    assert_eq!(
        obs["source"], "agent",
        "source defaults to 'agent' when not provided"
    );
}

// ── record_observation defaults confidence to 0.5 ────────────────

#[tokio::test]
async fn record_observation_defaults_confidence_to_half() {
    let (server, dir) = setup();
    let client = spawn_server(server).await;

    let _ = client
        .call_tool(
            CallToolRequestParams::new("record_observation").with_arguments(
                call_tool_args(json!({
                    "content": "Observation without explicit confidence",
                    "title": "Default confidence obs",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Verify the file on disk has confidence: 0.5
    let obs_dir = dir.path().join(".cogz").join("observations");
    let mut found = false;
    for entry in walkdir::WalkDir::new(&obs_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "md") {
            let content = std::fs::read_to_string(entry.path()).unwrap();
            assert!(
                content.contains("confidence: 0.5"),
                "observation file should have confidence: 0.5, got:\n{content}"
            );
            found = true;
        }
    }
    assert!(found, "expected at least one observation .md file");
}

// ── create_rule defaults confidence to 1.0 ────────────────────────

#[tokio::test]
async fn create_rule_defaults_confidence_to_1() {
    let (server, _dir) = setup();
    let client = spawn_server(server).await;

    let _ = client
        .call_tool(
            CallToolRequestParams::new("create_rule").with_arguments(
                call_tool_args(json!({
                    "content": "Rule without explicit confidence",
                    "title": "Default confidence rule",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Query it back and verify confidence defaults to 1.0
    let result = client
        .call_tool(
            CallToolRequestParams::new("query_rules")
                .with_arguments(call_tool_args(json!({})).unwrap()),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    let rule = &value["rules"][0];
    assert_eq!(
        rule["confidence"].as_f64(),
        Some(1.0),
        "confidence defaults to 1.0 when not provided"
    );
}

// ── update_knowledge with category change moves the file ──────────

#[tokio::test]
async fn update_knowledge_category_change_moves_file() {
    let (server, dir) = setup();
    let client = spawn_server(server).await;

    // Create knowledge in category "original"
    let result = client
        .call_tool(
            CallToolRequestParams::new("create_knowledge").with_arguments(
                call_tool_args(json!({
                    "title": "Movable knowledge",
                    "content": "Will be recategorized",
                    "category": "original",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let value = parse_result(result);
    let id = value["id"].as_str().unwrap();

    // Verify file is in the original category dir
    let cogz_dir = dir.path().join(".cogz");
    let original_dir = cogz_dir.join("knowledge").join("original");
    assert_eq!(
        std::fs::read_dir(&original_dir).unwrap().count(),
        1,
        "file exists in original category"
    );

    // Update with a new category
    let _ = client
        .call_tool(
            CallToolRequestParams::new("update_knowledge").with_arguments(
                call_tool_args(json!({
                    "id": id,
                    "content": "Updated content",
                    "category": "recategorized",
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();

    // Old file should be gone, new file should exist in new category
    assert_eq!(
        std::fs::read_dir(&original_dir).unwrap().count(),
        0,
        "old file removed from original category"
    );
    let new_dir = cogz_dir.join("knowledge").join("recategorized");
    assert_eq!(
        std::fs::read_dir(&new_dir).unwrap().count(),
        1,
        "file exists in new category"
    );
}
