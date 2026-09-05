//! Subprocess-based MCP integration test.
//!
//! Spawns the real `cogz mcp-stdio` binary and communicates over
//! stdio using JSON-RPC. This catches issues that the in-process
//! tests (test_mcp_server.rs) cannot — specifically main()-level
//! concerns like tracing subscriber configuration.
//!
//! All 13 tools are exercised over the real stdio transport against
//! a temporary repo initialized via `cogz init` + `cogz index`.
//! The MCP server runs with a fake HOME so ONNX models are not
//! found — search and get_context operate in FTS-only mode, which
//! is the graceful degradation path.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Path to the cogz binary, set by Cargo for integration tests.
const COGZ_BIN: &str = env!("CARGO_BIN_EXE_cogz");

/// The CogZ repo itself — used for read-only tests against a real
/// indexed database.
const REAL_REPO: &str = env!("CARGO_MANIFEST_DIR");

// ── JSON-RPC client ─────────────────────────────────────────────

/// Read one JSON-RPC line from stdout, skipping any non-JSON lines.
fn read_jsonrpc(reader: &mut BufReader<std::process::ChildStdout>) -> Option<serde_json::Value> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                serde_json::from_str(trimmed).ok()
            }
        }
        Err(_) => None,
    }
}

struct McpClient {
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn(env: &[(String, String)]) -> std::io::Result<(Self, std::process::Child)> {
        Self::spawn_with_repo(None, env)
    }

    fn spawn_with_repo(
        _repo: Option<&str>,
        env: &[(String, String)],
    ) -> std::io::Result<(Self, std::process::Child)> {
        let mut cmd = Command::new(COGZ_BIN);
        cmd.arg("mcp-stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok((
            Self {
                stdin,
                stdout,
                next_id: 1,
            },
            child,
        ))
    }

    fn send(&mut self, method: &str, params: serde_json::Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.stdin, "{}", msg).unwrap();
        self.stdin.flush().unwrap();
        id
    }

    fn send_notification(&mut self, method: &str) {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        writeln!(self.stdin, "{}", msg).unwrap();
        self.stdin.flush().unwrap();
    }

    /// Read the next JSON-RPC response, skipping non-JSON lines.
    fn recv(&mut self) -> Option<serde_json::Value> {
        for _ in 0..20 {
            if let Some(v) = read_jsonrpc(&mut self.stdout) {
                return Some(v);
            }
        }
        None
    }

    /// Initialize the MCP session. Must be called before tools/call.
    fn initialize(&mut self) {
        self.send(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1"},
            }),
        );
        self.recv().expect("no initialize response");
        self.send_notification("notifications/initialized");
    }

    /// Call a tool and return the response.
    fn call_tool(&mut self, name: &str, args: serde_json::Value) -> serde_json::Value {
        self.send(
            "tools/call",
            serde_json::json!({"name": name, "arguments": args}),
        );
        self.recv()
            .unwrap_or_else(|| panic!("no response for {name}"))
    }

    /// Get the text content from a tool response.
    fn tool_text(&mut self, name: &str, args: serde_json::Value) -> String {
        let resp = self.call_tool(name, args);
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    /// Check if a tool response is an error.
    fn tool_is_error(&mut self, name: &str, args: serde_json::Value) -> bool {
        let resp = self.call_tool(name, args);
        resp.get("error").is_some() || resp["result"]["isError"].as_bool().unwrap_or(false)
    }
}

// ── Temp repo setup ─────────────────────────────────────────────

/// A temporary repo with .cogz/ initialized and a few entity files.
struct TempRepo {
    dir: tempfile::TempDir,
    path: PathBuf,
}

impl TempRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().to_path_buf();

        // cogz init
        let status = Command::new(COGZ_BIN)
            .args(["init", "--repo"])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("failed to run cogz init");
        assert!(status.success(), "cogz init failed");

        // Add a knowledge file so search and queries have something to find.
        let knowledge_dir = path.join(".cogz/knowledge/architecture");
        std::fs::create_dir_all(&knowledge_dir).unwrap();
        std::fs::write(
            knowledge_dir.join("test-design.md"),
            "---\nid: 11111111-1111-1111-1111-111111111111\ntitle: \"Test Design\"\ntype: knowledge\nstatus: active\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\ncategory: architecture\ntags: [\"testing\"]\n---\n\nThis document describes the test architecture for the project.\nIt covers unit tests, integration tests, and end-to-end tests.\n",
        ).unwrap();

        // Add a rule file.
        let rules_dir = path.join(".cogz/rules");
        std::fs::create_dir_all(&rules_dir).unwrap();
        std::fs::write(
            rules_dir.join("always-test.md"),
            "---\nid: 22222222-2222-2222-2222-222222222222\ntitle: \"Always write tests\"\ntype: rule\nstatus: active\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\nconfidence: 0.9\n---\n\nEvery new feature must have tests before merging.\n",
        ).unwrap();

        // cogz index --no-download (FTS-only, no ONNX models needed)
        let status = Command::new(COGZ_BIN)
            .args(["index", "--no-download", "--repo"])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("failed to run cogz index");
        assert!(status.success(), "cogz index failed");

        Self { dir, path }
    }

    fn path_str(&self) -> &str {
        self.path.to_str().unwrap()
    }

    /// Environment for the MCP server: fake HOME so ONNX models are
    /// not found, forcing FTS-only mode for search/get_context.
    fn mcp_env(&self) -> Vec<(String, String)> {
        let fake_home = self.dir.path().join("fake-home");
        std::fs::create_dir_all(&fake_home).unwrap();
        vec![("HOME".to_string(), fake_home.to_str().unwrap().to_string())]
    }
}

// ── Tests: handshake and protocol ───────────────────────────────

#[test]
fn mcp_stdio_handshake_and_tools() {
    let (mut client, mut child) = McpClient::spawn(&[]).expect("failed to spawn cogz mcp-stdio");

    client.send(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0.1"},
        }),
    );
    let resp = client.recv().expect("no initialize response");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(
        resp["result"]["capabilities"]["tools"].is_object(),
        "capabilities should include tools"
    );

    client.send_notification("notifications/initialized");

    client.send("tools/list", serde_json::json!({}));
    let resp = client.recv().expect("no tools/list response");
    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools should be an array");
    assert_eq!(tools.len(), 13, "should expose 13 tools");

    // Verify repo is required in every tool schema
    for tool in tools {
        let required = tool["inputSchema"]["required"].as_array();
        assert!(
            required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some("repo"))),
            "tool {} should require repo",
            tool["name"].as_str().unwrap_or("?")
        );
    }

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_missing_repo_rejected() {
    let (mut client, mut child) = McpClient::spawn(&[]).expect("failed to spawn");
    client.initialize();

    let resp = client.call_tool("get_status", serde_json::json!({}));
    let is_error = resp["result"]["isError"].as_bool().unwrap_or(false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        is_error && text.contains("repo"),
        "missing repo should be rejected, got: is_error={is_error}, text={text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_invalid_repo_rejected() {
    let (mut client, mut child) = McpClient::spawn(&[]).expect("failed to spawn");
    client.initialize();

    let resp = client.call_tool(
        "get_status",
        serde_json::json!({"repo": "/nonexistent/path"}),
    );
    assert!(
        resp.get("error").is_some() || resp["result"]["isError"].as_bool().unwrap_or(false),
        "invalid repo should return error, got: {resp}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_tracing_goes_to_stderr_not_stdout() {
    let mut child = Command::new(COGZ_BIN)
        .arg("mcp-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    let messages = [
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "test", "version": "0.1"}}
        }),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "get_status", "arguments": {"repo": "/nonexistent/path"}}
        }),
    ];

    for msg in &messages {
        writeln!(stdin, "{}", msg).unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);

    let mut stdout_data = String::new();
    BufReader::new(stdout)
        .read_to_string(&mut stdout_data)
        .unwrap();

    for (i, line) in stdout_data.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        assert!(
            serde_json::from_str::<serde_json::Value>(line).is_ok(),
            "stdout line {i} is not valid JSON (tracing leaking to stdout?): {line}"
        );
    }

    let json_lines: Vec<_> = stdout_data
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect();
    assert!(
        json_lines.len() >= 2,
        "expected at least 2 JSON-RPC responses on stdout, got {}",
        json_lines.len()
    );

    assert!(
        json_lines[1].get("error").is_some()
            || json_lines[1]["result"]["isError"]
                .as_bool()
                .unwrap_or(false),
        "second response should be an error for invalid repo"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

// ── Tests: system tools (get_status, consolidate, capture_event) ──

#[test]
fn mcp_stdio_get_status() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text("get_status", serde_json::json!({"repo": repo.path_str()}));
    assert!(
        text.contains("db_path") && text.contains("entities"),
        "get_status should return status info, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_capture_event_session_start() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "capture_event",
        serde_json::json!({
            "repo": repo.path_str(),
            "event_type": "session_start",
        }),
    );
    // session_start returns a context pack
    assert!(
        text.contains("context_pack") || text.contains("sections") || text.contains("mode"),
        "capture_event session_start should return a context pack, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_capture_event_invalid_type() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let is_error = client.tool_is_error(
        "capture_event",
        serde_json::json!({
            "repo": repo.path_str(),
            "event_type": "invalid_event",
        }),
    );
    assert!(is_error, "invalid event_type should be rejected");

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_consolidate() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    // consolidate runs promotion + merge. On a fresh repo with no
    // duplicates, it should succeed with zero counts.
    let text = client.tool_text("consolidate", serde_json::json!({"repo": repo.path_str()}));
    assert!(
        text.contains("promoted") || text.contains("merged") || text.contains("consolidat"),
        "consolidate should return results, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

// ── Tests: query tools ──────────────────────────────────────────

#[test]
fn mcp_stdio_query_observations() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    // No observations in the temp repo, but the call should succeed.
    let text = client.tool_text(
        "query_observations",
        serde_json::json!({"repo": repo.path_str(), "limit": 10}),
    );
    assert!(
        text.contains("observations") || text.contains("count"),
        "query_observations should return results, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_query_rules() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    // The temp repo has one rule: "Always write tests"
    let text = client.tool_text("query_rules", serde_json::json!({"repo": repo.path_str()}));
    assert!(
        text.contains("rules") && text.contains("Always write tests"),
        "query_rules should return the test rule, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_query_knowledge() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    // The temp repo has one knowledge entry: "Test Design"
    let text = client.tool_text(
        "query_knowledge",
        serde_json::json!({"repo": repo.path_str()}),
    );
    assert!(
        text.contains("knowledge") && text.contains("Test Design"),
        "query_knowledge should return the test knowledge, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_list_entities() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "list_entities",
        serde_json::json!({
            "repo": repo.path_str(),
            "entity_type": "knowledge",
        }),
    );
    assert!(
        text.contains("entities") && text.contains("Test Design"),
        "list_entities should return the test knowledge, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

// ── Tests: write tools ──────────────────────────────────────────

#[test]
fn mcp_stdio_record_observation() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "record_observation",
        serde_json::json!({
            "repo": repo.path_str(),
            "title": "Test observation via stdio",
            "content": "This observation was created through the MCP stdio transport.",
            "source": "agent",
        }),
    );
    assert!(
        text.contains("id") || text.contains("created"),
        "record_observation should return an ID, got: {text}"
    );

    // Verify the file was created on disk
    let obs_dir = repo.path.join(".cogz/observations");
    assert!(
        obs_dir.exists() && obs_dir.read_dir().unwrap().count() > 0,
        "observation file should exist on disk"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_create_rule() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "create_rule",
        serde_json::json!({
            "repo": repo.path_str(),
            "title": "Test rule via stdio",
            "content": "Rules created through MCP must be tested.",
            "confidence": 0.8,
        }),
    );
    assert!(
        text.contains("id") || text.contains("created"),
        "create_rule should return an ID, got: {text}"
    );

    // Verify the file was created
    let rules_dir = repo.path.join(".cogz/rules");
    let count = rules_dir.read_dir().unwrap().count();
    assert!(
        count >= 2,
        "rule file should exist on disk (expected >= 2, got {count})"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_create_knowledge() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "create_knowledge",
        serde_json::json!({
            "repo": repo.path_str(),
            "title": "Test knowledge via stdio",
            "content": "Knowledge created through the MCP stdio transport.",
            "category": "architecture",
            "tags": ["testing", "mcp"],
        }),
    );
    assert!(
        text.contains("id") || text.contains("created"),
        "create_knowledge should return an ID, got: {text}"
    );

    // Verify the file was created
    let knowledge_dir = repo.path.join(".cogz/knowledge/architecture");
    let count = knowledge_dir.read_dir().unwrap().count();
    assert!(
        count >= 2,
        "knowledge file should exist on disk (expected >= 2, got {count})"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_update_knowledge() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    // Update the existing knowledge entry "Test Design"
    let text = client.tool_text(
        "update_knowledge",
        serde_json::json!({
            "repo": repo.path_str(),
            "id": "11111111-1111-1111-1111-111111111111",
            "content": "Updated content via MCP stdio transport. The test architecture now includes subprocess tests.",
            "category": "architecture",
        }),
    );
    assert!(
        text.contains("updated") || text.contains("id"),
        "update_knowledge should return success, got: {text}"
    );

    // Verify the file was updated
    let file_path = repo
        .path
        .join(".cogz/knowledge/architecture/test-design.md");
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        content.contains("Updated content via MCP stdio"),
        "knowledge file should contain updated content"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_update_knowledge_nonexistent_id() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let is_error = client.tool_is_error(
        "update_knowledge",
        serde_json::json!({
            "repo": repo.path_str(),
            "id": "99999999-9999-9999-9999-999999999999",
            "content": "This should fail.",
            "category": "architecture",
        }),
    );
    assert!(
        is_error,
        "update_knowledge with nonexistent ID should error"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

// ── Tests: search and context (FTS-only mode) ───────────────────

#[test]
fn mcp_stdio_search_fts_only() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "search",
        serde_json::json!({
            "repo": repo.path_str(),
            "query": "test architecture",
        }),
    );
    // FTS-only mode should find the "Test Design" knowledge entry
    assert!(
        text.contains("fts_only") || text.contains("results"),
        "search should return results in FTS-only mode, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_get_context_cold_start() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "get_context",
        serde_json::json!({
            "repo": repo.path_str(),
            "mode": "cold_start",
        }),
    );
    // cold_start doesn't need a query — returns a context pack
    assert!(
        text.contains("context_pack") || text.contains("sections") || text.contains("mode"),
        "get_context cold_start should return a context pack, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_get_context_task() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let text = client.tool_text(
        "get_context",
        serde_json::json!({
            "repo": repo.path_str(),
            "mode": "task",
            "query": "test architecture",
        }),
    );
    assert!(
        text.contains("context_pack") || text.contains("sections") || text.contains("mode"),
        "get_context task should return a context pack, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_get_context_invalid_mode() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let is_error = client.tool_is_error(
        "get_context",
        serde_json::json!({
            "repo": repo.path_str(),
            "mode": "invalid_mode",
        }),
    );
    assert!(is_error, "get_context with invalid mode should error");

    child.kill().unwrap();
    child.wait().unwrap();
}

// ── Tests: write + read round-trip ──────────────────────────────

#[test]
fn mcp_stdio_write_then_read_roundtrip() {
    let repo = TempRepo::new();
    let env = repo.mcp_env();
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    // Write an observation
    let write_text = client.tool_text(
        "record_observation",
        serde_json::json!({
            "repo": repo.path_str(),
            "title": "Round-trip test observation",
            "content": "This observation will be queried back through the stdio transport.",
            "source": "agent",
        }),
    );
    assert!(
        write_text.contains("id"),
        "record_observation should return an ID, got: {write_text}"
    );

    // Query it back
    let query_text = client.tool_text(
        "query_observations",
        serde_json::json!({"repo": repo.path_str(), "limit": 10}),
    );
    assert!(
        query_text.contains("Round-trip test observation"),
        "query_observations should find the created observation, got: {query_text}"
    );

    // Search for it
    let search_text = client.tool_text(
        "search",
        serde_json::json!({
            "repo": repo.path_str(),
            "query": "round-trip",
        }),
    );
    assert!(
        search_text.contains("results"),
        "search should return results for 'round-trip', got: {search_text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

// ── Tests: real repo (read-only) ─────────────────────────────────

/// Env vars for real-repo tests: fake HOME/LOCALAPPDATA so the MCP
/// server doesn't try to load ONNX models from the real user dir.
/// On CI, model dirs may not exist or model loading may fail, which
/// would cause the tool to error.
#[allow(unused_mut)]
fn real_repo_env() -> Vec<(String, String)> {
    let fake_home = std::env::temp_dir().join("cogz-test-fake-home");
    std::fs::create_dir_all(&fake_home).unwrap();
    let mut env = vec![("HOME".to_string(), fake_home.to_str().unwrap().to_string())];
    #[cfg(windows)]
    {
        let fake_appdata = std::env::temp_dir().join("cogz-test-fake-appdata");
        std::fs::create_dir_all(&fake_appdata).unwrap();
        env.push((
            "LOCALAPPDATA".to_string(),
            fake_appdata.to_str().unwrap().to_string(),
        ));
    }
    env
}

/// Ensure the real repo has a built DB. On CI, the checkout has no
/// `.cogz/cogz.db` — `cogz index --no-download` builds it from the
/// canonical Markdown files.
fn ensure_real_repo_indexed(env: &[(String, String)]) {
    let db_path = std::path::Path::new(REAL_REPO).join(".cogz/cogz.db");
    if db_path.exists() {
        return;
    }
    let status = Command::new(COGZ_BIN)
        .args(["index", "--no-download", "--repo"])
        .arg(REAL_REPO)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to run cogz index on real repo");
    assert!(
        status.success(),
        "cogz index --no-download failed on real repo"
    );
}

#[test]
fn mcp_stdio_real_repo_get_status() {
    let env = real_repo_env();
    ensure_real_repo_indexed(&env);
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let resp = client.call_tool("get_status", serde_json::json!({"repo": REAL_REPO}));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("db_path") && text.contains("entities"),
        "get_status on real repo should return status, got: {resp}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_real_repo_query_knowledge() {
    let env = real_repo_env();
    ensure_real_repo_indexed(&env);
    let (mut client, mut child) = McpClient::spawn(&env).expect("failed to spawn");
    client.initialize();

    let resp = client.call_tool("query_knowledge", serde_json::json!({"repo": REAL_REPO}));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    // The real CogZ repo has knowledge entries
    assert!(
        text.contains("knowledge") && text.contains("count"),
        "query_knowledge on real repo should return results, got: {resp}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}
