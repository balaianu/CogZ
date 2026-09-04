---
id: f8b0c1d2-09cc-414e-831c-8f5710374d4a
title: Manual transactions need ROLLBACK on error
type: rule
status: stale
created_at: "2026-09-03T12:10:00Z"
updated_at: "2026-09-04T09:12:04.108524067+00:00"
references: []
category: correctness
tags: ["transaction", "error-handling", "sqlite"]
confidence: 1
---

When using raw `BEGIN`/`COMMIT` on the shared SQLite connection,
every `?` early return inside the transaction body leaks the
transaction. The connection stays in transaction mode, breaking
all subsequent DB operations.

**Rule:** Wrap transactional operations in a closure. If the
closure returns an error, execute `ROLLBACK` before propagating.

**Reference pattern:** `merge_one` in `src/consolidate/merge.rs`.

**Why not rusqlite's Transaction guard:** The shared connection
behind `std::sync::Mutex` makes borrowing the guard across the
mutex boundary awkward. The closure pattern is simpler and equally
correct.
