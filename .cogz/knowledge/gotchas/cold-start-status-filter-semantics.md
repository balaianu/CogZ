---
id: b3c4d5e6-0002-4aaa-bbbb-000000000003
title: cold_start and search use opposite None semantics for status filtering
type: knowledge
status: active
created_at: 2026-08-28T19:40:00Z
updated_at: 2026-08-28T19:40:00Z
references: ["b3c4d5e6-0001-4aaa-bbbb-000000000002"]
category: gotchas
tags: ["status", "filtering", "cold_start", "search", "gotcha"]
---

The storage query layer and the search layer use opposite
semantics for `None` as a status filter:

- **`get_entities_by_type(conn, type, status, limit)`** —
  `None` means *no filter* (all statuses). `Some("active")`
  filters to active.

- **`search::search(conn, query, embedding, params, config)`** —
  `None` means *active only* (via `resolve_status_filter`).
  `Some("all")` means no filter.

This means context assembly must pass different values depending
on the code path:

```rust
// cold_start uses get_entities_by_type
let cold_start_status = if include_stale { None } else { Some("active") };

// task/escalation uses search::search
let search_status = if include_stale { Some("all") } else { None };
```

Getting this wrong causes stale entities to leak into cold_start
packs (or active entities to be excluded). The integration test
`include_stale_includes_stale_entities` catches this.
