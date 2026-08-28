---
id: ac43ee1b-2322-4ba1-afcf-cd2464a2d065
title: "Search pipeline — FTS, vector, RRF, graph expansion"
type: knowledge
status: active
created_at: 2026-08-28T19:22:00Z
updated_at: 2026-08-28T19:22:00Z
references: ["a9d8f4cd-a22a-4e0c-a25a-418c92564dcd"]
category: architecture
tags: ["search", "fts5", "vector", "rrf", "graph-expansion"]
---

The search pipeline in `src/search/hybrid.rs` runs six steps:

1. **FTS search** — `fts_search()` returns full entities ranked by
   FTS5's BM25. These are cached in `entity_map` to avoid
   re-fetching during fusion and vector filtering.

2. **Vector search** (optional) — `knn_search()` over-fetches
   (`limit * 3`) to compensate for post-KNN type/status filtering.
   Results are filtered in Rust, not SQL, because vec0 doesn't
   support JOIN with the entities table for filtering.

3. **RRF fusion** — `fuse()` is a pure function: takes ranked ID
   lists with weights, returns fused (id, score) pairs. No I/O.
   FTS-only mode skips fusion and uses rank-position scoring
   directly.

4. **Top-N selection** — take `params.limit` from the fused list.

5. **Direct results** — build `SearchResult` entries from
   `entity_map` (no additional DB fetches for FTS-matched entities).

6. **Graph expansion** (optional) — BFS from seed IDs using
   `get_edges_involving_batch` (one query per hop). Path
   descriptions are batch-built in `describe.rs` (2 queries total
   for all paths, not 2N-1 per path).

The `SearchConfig.max_results` field from `config.toml` is the
default limit when `--limit` is not specified on the CLI.
