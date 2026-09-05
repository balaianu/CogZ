---
id: d6eafb0c-09cc-414e-831c-8f5710374d4a
title: KNN returns embeddings for non-active entities
type: observation
status: stale
created_at: "2026-09-03T12:06:00Z"
updated_at: "2026-09-04T09:05:09.907375497+00:00"
references: []
source: agent
confidence: 0.9
supporting_ids: []
---

The vec0 embedding table retains embeddings for entities after
their status changes. A rejected, superseded, or pruned entity's
embedding is still in vec0 and will be returned by KNN search.

This is by design — removing embeddings on status change would be
destructive (the entity might transition back: stale → active).
But it means any consumer of KNN results must filter by status
after retrieval, not assume the results are all active.

## Where this matters

- `check_duplicate` in `src/consolidate/dedup.rs`: KNN neighbors
  must be filtered against the active entity list
- `expand_with_paths` in `src/search/expand.rs`: already filters
  by status via `batch_check_status` — this was correct
- Context assembly: uses search results which are already
  status-filtered — this was correct

## What would break if we removed embeddings on status change

- `stale → active` transitions would need re-embedding (expensive,
  and the content hasn't changed — the embedding is still valid)
- Tombstoned entities would lose their embedding permanently —
  if a tombstone is ever un-tombstoned (currently not supported,
  but the schema allows it), the embedding would need recomputation
- The vec0 table would need DELETE operations, which are more
  expensive than UPDATEs in sqlite-vec

The current design (keep embeddings, filter at query time) is
correct and performant.
