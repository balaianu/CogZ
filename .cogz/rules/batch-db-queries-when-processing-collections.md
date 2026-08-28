---
id: ac57f6c8-0669-4062-a9f5-315b419f7c73
title: Batch DB queries when processing collections
type: rule
status: active
created_at: 2026-08-28T19:32:00Z
updated_at: 2026-08-28T19:32:00Z
references: []
confidence: 1.0
validation_count: 3
supporting_ids: []
---

When processing a collection of items that each need a SQL query,
batch them into a single query using `IN (?, ?, ...)` instead of
one query per item.

**Reference implementations:**

- `get_neighbors_batch` — fetches all neighbors for a frontier in
  2 queries (outgoing + incoming), not N queries per frontier node.

- `get_edges_involving_batch` — fetches all edges touching a node
  set in 1 query, returning (source, target, edge_type) triples.

- `build_path_descriptions_batch` — fetches all titles and all
  edge types for multiple paths in 2 queries total.

**Pattern for building placeholders:**

```rust
let placeholders = (0..ids.len())
    .map(|_| "?")
    .collect::<Vec<_>>()
    .join(",");
let params: Vec<&dyn rusqlite::ToSql> =
    ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
let sql = format!("SELECT ... WHERE id IN ({placeholders})");
```

**When params are bound multiple times** (e.g., `source_id IN (...)
OR target_id IN (...)`), chain the params:

```rust
let params: Vec<&dyn rusqlite::ToSql> = ids
    .iter()
    .chain(ids.iter())
    .map(|s| s as &dyn rusqlite::ToSql)
    .collect();
```

Apply this proactively to any new code that loops over a
collection and queries the DB per item.
