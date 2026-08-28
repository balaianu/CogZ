---
id: a9d8f4cd-a22a-4e0c-a25a-418c92564dcd
title: "Module layering and data ownership boundaries"
type: knowledge
status: active
created_at: 2026-08-28T19:20:00Z
updated_at: 2026-08-28T19:20:00Z
references: []
category: architecture
tags: ["layering", "architecture", "ownership", "invariants"]
---

CogZ has four layers with strict ownership boundaries:

1. **storage/** — owns all SQL. No other module writes to the
   database. The `Connection` is behind a `Mutex` inside `Storage`.
   All functions take `&Connection`, not `&Storage`, so callers
   acquire the lock and pass the guard.

2. **files/** — owns canonical file I/O. Files are the source of
   truth; the DB is derived. Sync is one-directional (file → DB).
   The `embed_sync` submodule bridges files and embeddings without
   holding the DB lock during model inference.

3. **embed/** — owns model inference. The `EmbeddingModel` trait
   abstracts over ONNX and mock implementations. The cache is
   content-hash based and in-memory (not persistent across
   processes).

4. **search/** — owns query orchestration. It takes an optional
   query embedding and degrades to FTS-only when none is provided.
   Search never loads models — that's the CLI's job.

The CLI boundary (cli.rs, main.rs) is the only place that touches
all four layers. It owns model loading, query embedding, and
user-facing output.

The key invariant: no layer reaches below the one it depends on.
Search calls storage, not the other way around. Files call storage,
not search. This makes each layer testable in isolation.
