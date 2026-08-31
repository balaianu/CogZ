---
id: a7c8e9f0-1234-5678-9abc-def012345678
type: knowledge
category: decision
title: bge-small-en-v1.5 as default embedding model
tags: [embedding, model, performance, onnx]
created_at: 2026-08-31T19:45:00Z
updated_at: 2026-08-31T19:45:00Z
---

# Decision: bge-small-en-v1.5 as default embedding model

## Context

The original design used CodeRankEmbed (768d, code-specific) for code
entities and bge-base-en-v1.5 (768d) for knowledge entities. Both
models produced high-quality embeddings but were too slow for CPU-only
inference on a ~1000-entity repo.

## Problem

CodeRankEmbed-int8 took ~25-35 minutes to embed 1014 code entities on
CPU, with RSS climbing to 5GB. This is unacceptable for a tool that
should feel responsive. FastEmbed (Python) with bge-small achieved
~35ms per embedding in batch mode — 3x faster than the Rust
implementation with the 768d model.

## Decision

Switch the default model to `BAAI/bge-small-en-v1.5` (384d, 64MB) for
both code and knowledge embeddings. Use Qdrant's graph-optimized ONNX
export (`Qdrant/bge-small-en-v1.5-onnx-Q`).

## Rationale

- 384d vectors are 2x smaller in storage and 2x faster in KNN search
- 64MB model downloads in seconds vs minutes
- Inference is ~3x faster than 768d models on CPU
- RSS stays under 500MB during background embedding (vs 5GB with
  CodeRankEmbed at batch size 64)
- Background embedding of 1014 entities completes in ~25 minutes
- Foreground index returns in ~3 minutes (code embedding deferred)

## Trade-offs

- bge-small is a general-purpose model, not code-specific. Code search
  quality may be slightly lower than CodeRankEmbed.
- 384d captures less semantic nuance than 768d.
- Both trade-offs are acceptable for a local-first tool where FTS
  handles exact-match queries and vector search supplements them.

## Alternatives preserved

CodeRankEmbed and bge-base remain in the registry as config options.
Users who need higher quality and can afford the compute cost can
switch by editing `.cogz/config.toml` and changing the dimension.

## Schema impact

The vec0 virtual table dimension is now configurable (passed to
`Storage::open` and `run_migrations`). Previously it was hardcoded to
768. Changing models requires `cogz reset` + `cogz index` to rebuild
the DB with the new dimension.
