---
id: 0b83613e-61d6-4a60-af20-2e377d72e4f7
title: No mutex reentrancy in storage helpers
type: rule
status: stale
created_at: "2026-08-29T02:50:00Z"
updated_at: "2026-09-06T10:31:59.474014380+00:00"
references: []
confidence: 0.95
---

Any function that internally calls `storage.conn()` (acquiring the
SQLite mutex) must not be called while the mutex is already held.
`std::sync::Mutex` is not reentrant — a second `conn()` call from the
same thread will deadlock.

Before calling any `Storage` method that acquires the mutex internally
(e.g. `db_size_bytes`), drop the current `conn` guard first. If you need
data from the DB and then need to call such a method, fetch the data,
drop the guard, call the method, then re-acquire if needed.
