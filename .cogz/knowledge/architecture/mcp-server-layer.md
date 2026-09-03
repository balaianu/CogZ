---
id: 4cc109e1-09cc-414e-831c-8f5710374d4a
title: MCP Server Layer
type: knowledge
status: active
created_at: "2026-08-29T02:30:00Z"
updated_at: "2026-09-03T11:50:00Z"
references: []
category: architecture
tags: ["mcp", "architecture", "rmcp", "async"]
---

# MCP Server Layer

The MCP server exposes CogZ's storage, search, and context capabilities
to AI agents via the Model Context Protocol over stdio transport.

## Module structure

- `src/mcp/mod.rs` — module declarations, re-exports `CogzServer`
- `src/mcp/server.rs` — `CogzServer` struct, `ServerHandler` impl,
  `run_stdio()` entry point
- `src/mcp/tools.rs` — `#[tool_router]` impl with 13 `#[tool]` methods
- `src/mcp/params.rs` — parameter structs (`serde::Deserialize` +
  `schemars::JsonSchema`)
- `src/mcp/helpers.rs` — file-first write logic, response builders,
  error helpers, embedding helper. Uses `sync_single_file` for O(1)
  file synchronization on writes (not full incremental scan).
- `src/mcp/dedup.rs` — title match + embedding similarity dedup.
  Filters to active entities only — rejected/superseded/pruned
  entities don't trigger duplicate warnings.
- `src/mcp/update_knowledge.rs` — in-place knowledge updates with
  file move support on category change. Uses `sync_single_file`.

## Key patterns

**Async/sync boundary:** All DB operations go through
`tokio::task::spawn_blocking`. Each tool handler clones `Arc<Storage>`,
`Config`, and `cogz_dir` before entering the blocking closure. The
SQLite mutex is never held during filesystem I/O.

**File-first invariant:** Write tools (record_observation, create_rule,
create_knowledge, update_knowledge) write the Markdown file first, then
sync to DB, then run dedup checks. If the file write fails, the DB is
not touched.

**Graceful degradation:** The `embed_query_for_search` helper returns
`None` when the ONNX model is unavailable. Search falls back to FTS-only,
dedup falls back to title-only matching.

**rmcp macro usage:** `#[tool_router(vis = "pub")]` generates a public
`tool_router()` method. `#[tool_handler]` on the `ServerHandler` impl
generates the `call_tool` dispatch. The `vis = "pub"` is required because
`tool_handler` and `tool_router` are in different modules.

## What's deferred

- Code entities are indexed by `cogz index` (Phase 8) and searchable
  via the `search` and `list_entities` tools, but there are no
  MCP tools for direct code entity manipulation — they're read-only
  products of the indexer.
