---
id: c9d3e4f5-09cc-414e-831c-8f5710374d4a
title: Dedup must filter to active entities only
type: knowledge
status: stale
created_at: "2026-09-03T11:57:00Z"
updated_at: "2026-09-06T20:50:06.005718566+00:00"
references: []
category: gotchas
tags: ["dedup", "consolidation", "status-filter", "knn"]
---

# Dedup must filter to active entities only

`check_duplicate` originally called `get_entities_by_type` with
`None` for the status filter, which includes all entities — active,
stale, rejected, superseded, and pruned. A new observation could be
flagged as a duplicate of a rejected or pruned entity.

This is wrong: a new observation that matches a rejected one is not
a duplicate — the rejected one was explicitly discarded. Matching
against superseded entities is also wrong since they've been replaced.

## Two places to filter

1. **Title match check:** The `existing` list passed to
   `check_title_match` must be filtered to `Some("active")`.

2. **KNN embedding similarity:** `knn_search` returns neighbors from
   the vec0 table, which still contains embeddings for stale,
   rejected, and superseded entities (they're not removed on status
   change). Each KNN neighbor must be checked against the active
   `existing` list — if it's not in the list, skip it.

## The fix

```rust
let existing = get_entities_by_type(conn, entity_type, Some("active"), 100)?;
// ...
for (entity_id, distance) in &neighbors {
    let Some(entity) = existing.iter().find(|e| e.id == *entity_id) else {
        continue;  // skip non-active neighbors
    };
    // ... similarity check
}
```

## Why embeddings persist across status changes

Removing embeddings on status change would be destructive — the
entity might transition back (stale → active). The vec0 table is
append-only during normal operation. This is why the KNN filter is
necessary.
