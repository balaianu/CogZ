---
id: 12b7be64-e950-4f69-99f2-ceb31da698b6
title: MCP 2026-07-28 spec alignment
type: knowledge
status: active
created_at: "2026-09-05T07:07:11.328112049+00:00"
updated_at: "2026-09-05T07:07:11.328112049+00:00"
references: ["src/mcp/server.rs", "src/main.rs"]
category: architecture
tags: ["mcp", "protocol", "spec", "alignment"]
---

The 2026-07-28 MCP specification (SEP-2577) deprecated Roots, Sampling, and Logging. Roots is replaced by passing paths via tool parameters, resource URIs, or server configuration. Sessions were removed (SEP-2567/2575) — the initialize/initialized handshake and Mcp-Session-Id are gone from the protocol core. Every request is now self-contained.

CogZ's design was already aligned before the spec caught up:
- Every tool requires an explicit `repo` parameter (no Roots, no implicit session context)
- Tracing goes to stderr in stdio mode (matches Logging deprecation guidance)
- No fallback repository, no cwd inference
- Each tool call is self-contained given `repo`

The cyanheads/filesystem-mcp-server added a `set_filesystem_default` tool as a session-scoped workaround — that pattern is now dead per the stateless spec. CogZ's explicit `repo` parameter approach is simpler and future-proof.

Protocol version was upgraded from V_2024_11_05 to ProtocolVersion::LATEST (V_2025_11_25) in rmcp 3.1.4. rmcp negotiates down to older clients, so backward compatibility is preserved. V_2026_07_28 exists in rmcp but LATEST points to V_2025_11_25, indicating rmcp considers it the stable default for stdio. The stateless changes in V_2026_07_28 primarily affect HTTP transports; stdio is unaffected.