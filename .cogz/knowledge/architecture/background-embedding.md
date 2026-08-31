---
id: c9e0a1b2-3456-789a-bcde-f23456789012
type: knowledge
category: architecture
title: Background code embedding after index
tags: [embedding, index, performance, background]
created_at: 2026-08-31T19:55:00Z
updated_at: 2026-08-31T19:55:00Z
---

# Background code embedding after index

## Overview

The `cogz index` command embeds knowledge entities (observations,
rules, knowledge files) inline but defers code entity embedding to a
background process. This keeps the foreground command responsive
(~3 minutes) while code embedding (~25 minutes for 1014 entities)
continues asynchronously.

## How it works

1. `cogz index` syncs all files and code entities to the DB
2. Knowledge entities are embedded inline (29 entities, fast)
3. Code entity IDs are written to a temp file (`.cogz/embed-bg-ids.txt`)
4. A child process is spawned: `cogz embed-bg --db ... --ids-file ...`
5. The foreground command returns immediately
6. The background process reads IDs, fetches entity data, embeds in
   chunks of 32, and stores vectors after each chunk

## Design constraints

- The background process opens its own `Storage` connection (the
  parent's mutex is not shared across processes)
- Entity data is fetched under the lock, then the lock is dropped
  before inference (no mutex held during ONNX execution)
- Embeddings are stored in chunks of 32 to bound memory usage
- The IDs file is cleaned up on completion (or on early exit if no
  entities need embedding)

## Failure handling

If the background process fails (model unavailable, inference error),
it logs warnings and exits. The IDs file may remain on disk. A
subsequent `cogz index` or `cogz reindex` will re-embed missing
entities.

## Search during background embedding

Search works immediately after the foreground command returns. FTS
results are complete. Vector results appear incrementally as
embeddings are stored. This is acceptable — the user gets immediate
FTS results and progressively better vector results.
