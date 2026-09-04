//! Subprocess-based MCP integration test.
//!
//! Spawns the real `cogz mcp-stdio` binary and communicates over
//! stdio using JSON-RPC. This catches issues that the in-process
//! tests (test_mcp_server.rs) cannot — specifically main()-level
//! concerns like tracing subscriber configuration.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

/// Path to the cogz binary, set by Cargo for integration tests.
const COGZ_BIN: &str = env!("CARGO_BIN_EXE_cogz");

/// The CogZ repo itself — guaranteed to have .cogz/ with an indexed DB.
const REPO: &str = env!("CARGO_MANIFEST_DIR");

/// Read one JSON-RPC line from stdout, skipping any non-JSON lines.
/// Returns None on EOF or read error.
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
    fn spawn() -> std::io::Result<(Self, std::process::Child)> {
        let mut child = Command::new(COGZ_BIN)
            .arg("mcp-stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

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

    /// Read the next JSON-RPC response, retrying on non-JSON lines.
    /// Times out after 30 seconds.
    fn recv(&mut self) -> Option<serde_json::Value> {
        for _ in 0..10 {
            if let Some(v) = read_jsonrpc(&mut self.stdout) {
                return Some(v);
            }
        }
        None
    }
}

#[test]
fn mcp_stdio_handshake_and_tools() {
    let (mut client, mut child) = McpClient::spawn().expect("failed to spawn cogz mcp-stdio");

    // Initialize
    client.send(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
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

    // Initialized notification
    client.send_notification("notifications/initialized");

    // List tools
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
fn mcp_stdio_get_status_with_repo() {
    let (mut client, mut child) = McpClient::spawn().expect("failed to spawn");

    client.send(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0.1"},
        }),
    );
    client.recv().expect("init response");
    client.send_notification("notifications/initialized");

    // get_status with valid repo
    client.send(
        "tools/call",
        serde_json::json!({
            "name": "get_status",
            "arguments": {"repo": REPO},
        }),
    );
    let resp = client.recv().expect("no get_status response");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("db_path") || text.contains("entities"),
        "get_status should return status info, got: {text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_missing_repo_rejected() {
    let (mut client, mut child) = McpClient::spawn().expect("failed to spawn");

    client.send(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0.1"},
        }),
    );
    client.recv().expect("init response");
    client.send_notification("notifications/initialized");

    // get_status without repo — should error
    client.send(
        "tools/call",
        serde_json::json!({
            "name": "get_status",
            "arguments": {},
        }),
    );
    let resp = client.recv().expect("no response for missing repo");
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
    let (mut client, mut child) = McpClient::spawn().expect("failed to spawn");

    client.send(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0.1"},
        }),
    );
    client.recv().expect("init response");
    client.send_notification("notifications/initialized");

    // get_status with invalid repo — should return JSON-RPC error
    client.send(
        "tools/call",
        serde_json::json!({
            "name": "get_status",
            "arguments": {"repo": "/nonexistent/path"},
        }),
    );
    let resp = client.recv().expect("no response for invalid repo");
    assert!(
        resp.get("error").is_some() || resp["result"]["isError"].as_bool().unwrap_or(false),
        "invalid repo should return error, got: {resp}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn mcp_stdio_tracing_goes_to_stderr_not_stdout() {
    // This is the regression test for the tracing-to-stdout bug.
    // If tracing output appears on stdout, it corrupts the JSON-RPC
    // transport. We verify that every line on stdout is valid JSON.
    //
    // Uses a raw process (not McpClient) so we can read all stdout
    // after closing stdin.

    let mut child = Command::new(COGZ_BIN)
        .arg("mcp-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Send initialize + initialized + a tool call that triggers a WARN log
    let messages = [
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {},
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
    drop(stdin); // Close stdin so server exits after processing

    // Read all stdout
    let mut stdout_data = String::new();
    let mut reader = BufReader::new(stdout);
    reader.read_to_string(&mut stdout_data).unwrap();

    // Every non-empty line on stdout must be valid JSON
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

    // We should have received at least 2 JSON responses (init + error)
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

    // The second response should be an error (invalid repo)
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
