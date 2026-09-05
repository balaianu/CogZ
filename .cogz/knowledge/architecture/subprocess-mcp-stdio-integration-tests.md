---
id: f32a2e47-512c-47d7-83e9-3832cff3cff7
title: Subprocess MCP stdio integration tests
type: knowledge
status: stale
created_at: "2026-09-05T07:07:18.941342516+00:00"
updated_at: "2026-09-05T13:02:42.939057278+00:00"
references: ["tests/test_mcp_stdio.rs"]
category: architecture
tags: ["testing", "mcp", "subprocess", "integration"]
---

The subprocess MCP integration tests in tests/test_mcp_stdio.rs spawn the real cogz binary and communicate over stdio using JSON-RPC. This catches issues that in-process tests (test_mcp_server.rs) cannot — specifically main()-level concerns like tracing subscriber configuration.

Test infrastructure:
- TempRepo helper creates a temp dir, runs cogz init + cogz index --no-download, seeds knowledge/rule files
- McpClient helper provides JSON-RPC client with initialize(), call_tool(), tool_text(), tool_is_error()
- Fake HOME env var forces FTS-only mode (no ONNX model loading), keeping tests fast (~12s total)

24 tests cover all 13 tools over stdio:
- Handshake: initialize, tools/list (13 tools, repo required in all schemas)
- Error handling: missing repo, invalid repo, invalid event_type, invalid context mode, nonexistent knowledge ID
- Write tools: record_observation, create_rule, create_knowledge, update_knowledge (verifies files on disk)
- Query tools: query_observations, query_rules, query_knowledge, list_entities
- Search/context: search (FTS-only), get_context cold_start + task + invalid mode
- System tools: get_status, consolidate, capture_event session_start + invalid type
- Round-trip: write observation → query it back → search for it
- Real repo: get_status + query_knowledge against CogZ's own DB
- Regression: tracing stays on stderr (no stdout corruption)

Key finding: the tracing-to-stdout bug was only detectable through subprocess testing. In-process tests bypass main() and its tracing setup, so they passed while the real binary was broken.