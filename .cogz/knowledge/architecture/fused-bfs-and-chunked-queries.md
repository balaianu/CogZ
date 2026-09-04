---
id: b3c4d5e6-f789-4abc-def0-123456789003
title: Fused multi-seed BFS and chunked SQL queries
type: knowledge
status: stale
created_at: "2026-08-29T23:20:00Z"
updated_at: "2026-09-04T09:05:09.805742301+00:00"
references: []
category: architecture
tags: ["graph-expansion", "bfs", "sql", "performance", "phase-8"]
---

## Fused multi-seed BFS

Graph expansion in `search/expand.rs` uses a fused multi-seed BFS
instead of running separate BFS per seed:

- One shared frontier across all seed entities
- One `get_edges_involving_batch` query per hop (not per seed)
- Each result records its first-reaching path and associated `seed_id`
- Status filtering is batched via `batch_check_status`

This reduces the number of SQL queries from O(seeds × hops) to
O(hops). For a search with 20 seeds and 3 hops, that's 3 queries
instead of 60.

## Chunked IN(...) queries

SQLite has a variable limit (`SQLITE_MAX_VARIABLE_NUMBER`, typically
999 or 32798 depending on version). Large batch queries are chunked
to stay below this limit:

- `get_edges_involving_batch`: chunk size 49 (IDs bound twice + extra
  params)
- `batch_check_status`: chunk size 998 (IDs + status param)
- `get_references_batch`: chunk size 999
- `get_neighbors_batch`: chunk size 999
- `get_entities_batch`: chunk size 999

Each chunk builds placeholders dynamically:
`(0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",")`

This prevents "too many SQL variables" errors on large repos with
hundreds of code entities.
