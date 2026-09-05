---
id: e1606dc8-e66d-454d-a665-c1508f4fb493
title: Subprocess tests required for MCP stdio servers
type: rule
status: active
created_at: "2026-09-05T07:08:25.935098836+00:00"
updated_at: "2026-09-05T07:08:25.935098836+00:00"
references: ["tests/test_mcp_stdio.rs"]
confidence: 0.85
---

Subprocess integration tests that spawn the real binary are necessary to catch main()-level issues that in-process tests miss. The tracing-to-stdout bug was only detectable through subprocess testing — in-process tests bypass main() and its tracing setup. Every MCP server should have both in-process tests (for tool logic) and subprocess tests (for transport and initialization). The subprocess tests should verify: protocol handshake, tool listing, error handling, and that stdout contains only valid JSON (no tracing leakage).