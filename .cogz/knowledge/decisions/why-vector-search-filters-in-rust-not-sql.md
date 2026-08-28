---
id: 2139737d-c249-4ac9-ba08-f6880298216d
title: "Why vector search filters in Rust, not SQL"
type: knowledge
status: active
created_at: 2026-08-28T19:24:00Z
updated_at: 2026-08-28T19:24:00Z
references: ["ac43ee1b-2322-4ba1-afcf-cd2464a2d065"]
category: decisions
tags: ["vec0", "sqlite-vec", "filtering", "trade-off"]
---

sqlite-vec's vec0 virtual table doesn't support JOINs with regular
tables. This means you can't do:

```sql
SELECT e.* FROM entity_embeddings v
JOIN entities e ON e.id = v.entity_id
WHERE v.embedding MATCH ? AND k = ?
  AND e.type = 'observation' AND e.status = 'active'
```

The vec0 KNN query runs in isolation. Type and status filtering
must happen after retrieving the KNN results.

**Decision:** Over-fetch (`limit * 3`) and filter in Rust. The
filtered results are capped at `limit` after filtering.

**Trade-off:** If most embeddings fail the filter, the result set
could be smaller than `limit`. A retry-with-larger-k mechanism
would fix this, but the current heuristic works for repos where
most entities are active and the type filter is selective enough.

**Future concern:** When Phase 8 adds code entities with
CodeRankEmbed embeddings, a single KNN query with a bge-base query
vector against CodeRankEmbed code embeddings would be semantically
incorrect — different embedding spaces. The search function will
need to run separate KNN queries per model and merge, as
architecture.md specifies.
