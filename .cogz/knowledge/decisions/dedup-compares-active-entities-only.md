---
id: b4c8d9ea-09cc-414e-831c-8f5710374d4a
title: Dedup compares active entities only
type: knowledge
status: stale
created_at: "2026-09-03T12:02:00Z"
updated_at: "2026-09-05T09:13:27.587455885+00:00"
references: []
category: decisions
tags: ["dedup", "consolidation", "status-filter", "design-decision"]
---

# Decision: Dedup compares active entities only

## Context

`check_duplicate` runs on every `record_observation`, `create_rule`,
and `create_knowledge` call. It compares the new entity against
existing entities by title similarity and embedding similarity.

The original implementation queried all entities regardless of
status. This meant a new observation could be flagged as a duplicate
of a rejected observation (one that was explicitly discarded) or a
pruned entity (one that was tombstoned).

## Decision

Only compare against entities with `status = "active"`. Stale,
rejected, superseded, and pruned entities are excluded from both:

1. The title match candidate list (`get_entities_by_type` with
   `Some("active")`)
2. The KNN embedding similarity results (filter KNN neighbors
   against the active list)

## Rationale

- A new observation matching a **rejected** one is not a duplicate —
  the rejection was an explicit decision. The new one might be a
  fresh attempt or a different take.
- A new observation matching a **superseded** one is not a duplicate
  — the superseded one was replaced by a better version. The new
  one should be compared against the survivor, not the obsolete
  entry.
- A new observation matching a **pruned** one is not a duplicate —
  pruned entities are tombstones with no content. Matching is
  meaningless.
- A new observation matching a **stale** one is ambiguous — stale
  means the referenced code changed. Including stale entities would
  produce confusing warnings about entities that may not even be
  relevant anymore.

## What about consolidation?

`run_consolidation` (the batch job) uses its own candidate selection
which also filters by status. The `find_duplicates` function in
`src/consolidate/dedup.rs` only considers active entities. This
decision aligns the inline `check_duplicate` (per-insert) with the
batch `find_duplicates` (consolidation run).
