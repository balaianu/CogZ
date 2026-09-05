---
id: c4d5e6f7-89ab-4cde-f012-345678900111
title: "Model path bug — OnnxEmbeddingModel::new vs with_model_id"
type: observation
status: stale
created_at: "2026-08-29T23:25:00Z"
updated_at: "2026-09-04T12:56:48.653515229+00:00"
references: []
source: agent
confidence: 0.9
tags: ["bug", "embedding", "config", "phase-8-audit"]
---

`run_status` in `main.rs` and `embed_query` in `cli.rs` were using
`OnnxEmbeddingModel::new()` (which uses hardcoded default model
directory paths) instead of `with_model_id()` (which uses the
configured `code_model` and `knowledge_model` from config.toml).

**Impact:** If a user configured custom model paths in
`[embedding].code_model` or `[embedding].knowledge_model`, `cogz
status` would report wrong model availability (checking default paths
instead of configured ones), and `cogz search` would embed queries
with the default model instead of the configured one — producing
mismatched embeddings if a custom knowledge model was configured.

**Root cause:** The `with_model_id` method was added in Phase 8 for
multi-model support, but not all call sites were updated. The MCP
server (`mcp/server.rs`) and status builder (`mcp/status.rs`) were
correctly updated, but the CLI paths were missed.

**Fix:** Both `run_status` (now in `commands.rs`) and `embed_query`
in `cli.rs` now use `with_model_id()` with the configured model IDs.
`run_status` also now prints the configured model names instead of
hardcoded "CodeRankEmbed" and "bge-base".
