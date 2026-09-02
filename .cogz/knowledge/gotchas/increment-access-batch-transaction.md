---
id: a1b2c3d4-5e6f-4a7b-8c9d-0e1f2a3b4c5d
title: increment_access_batch must use a transaction
type: knowledge
category: gotchas
tags: [performance, sqlite, transactions]
created_at: 2026-09-02T16:00:00Z
updated_at: 2026-09-02T16:00:00Z
status: active
---

`increment_access_batch` in `src/storage/access.rs` must wrap its
INSERT/UPDATE loop in a single transaction. Without it, each statement
triggers a separate ext4 journal commit — 366 ids = 366 commits = ~15
seconds of disk sleep state.

With `conn.unchecked_transaction()` wrapping the loop, the same 366
ids complete in ~70ms. That's a 200x improvement from one line of code.

This is the same class of bug that affected Mnemos: per-row writes
without transaction batching cause pathological I/O on journaling
filesystems. Any batch write to SQLite must use a transaction.
