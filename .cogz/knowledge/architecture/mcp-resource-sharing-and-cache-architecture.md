---
id: a77468ff-07d1-451d-a14d-702760586875
title: MCP resource sharing and cache architecture
type: knowledge
status: stale
created_at: "2026-09-05T07:07:28.257259896+00:00"
updated_at: "2026-09-06T10:31:59.920890297+00:00"
references: []
category: architecture
tags: ["mcp", "cache", "resources", "concurrency"]
---

The MCP server uses a multi-layer resource sharing architecture to ensure that no matter how many hooks, CLI calls, or MCP tools fire, they share the same resources without duplicates.

Repository cache (src/mcp/repo_cache.rs):
- Per-key Opening locks prevent thundering-herd duplicate opens. The first caller claims the key; others wait on a per-key mutex.
- Bounded LRU eviction: max 8 ready entries, tracked by access order vector (front = LRU, back = MRU).
- Config staleness: stores config.toml mtime; on cache hit, checks current mtime. Changed config → evict and reopen.
- Lock ordering: always repos first, then lru_order, to prevent deadlock.

Model cache:
- Embedding models keyed by (model_id, dimension, ModelType). Two repos with the same model share one ONNX session.
- NLI models keyed by model_id only.
- Second cache check before insertion: if another caller inserted first, the newly created duplicate is discarded and the existing Arc is returned.

Mutex handling:
- All MCP mutex locks use poison recovery: .lock().unwrap_or_else(|e| e.into_inner())
- This matches Storage::conn() behavior and prevents a panicked thread from bricking the server.

Verified: 543 tests pass, including 24 subprocess tests that exercise the real binary over stdio.