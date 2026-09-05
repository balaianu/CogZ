---
id: 0644e3bb-3600-42ef-a7a7-3c1bbd01e119
title: "Embedding sync lock pattern — fetch, drop, infer, re-acquire"
type: knowledge
status: stale
created_at: "2026-08-28T19:30:00Z"
updated_at: "2026-09-04T12:59:11.495518097+00:00"
references: ["a9d8f4cd-a22a-4e0c-a25a-418c92564dcd"]
category: gotchas
tags: ["mutex", "concurrency", "embeddings", "onnx", "gotcha"]
---

The `embed_synced` function in `src/cli.rs` follows a three-phase
pattern to avoid holding the SQLite mutex during ONNX inference:

1. **Fetch under lock** — acquire `storage.conn()`, read entity
   data into `Vec<Entity>`, drop the lock guard (end of block
   scope).

2. **Infer without lock** — call `embed_entities()` which runs
   ONNX inference. This can take seconds per entity. No DB access.

3. **Store under lock** — re-acquire `storage.conn()`, call
   `store_embeddings()` which does `DELETE` + `INSERT` into vec0.

**Why this matters:** ONNX inference on a 2012-era FX-8320 can
take 50-200ms per text. If the lock is held during inference, all
other DB callers (search, sync, status) are blocked. In an MCP
server context (Phase 7), this would freeze the entire server
during indexing.

**The reference pattern:** `db_size_bytes` in `storage/mod.rs`
does the same thing for filesystem metadata — query the path under
lock, drop the lock, call `std::fs::metadata` outside the lock.

**Gotcha:** The `search` function holds the lock for its entire
duration. This is correct — search is pure DB reads with no I/O.
But if search ever needs to call an external reranker, it must
follow the same fetch-drop-infer-re-acquire pattern.
