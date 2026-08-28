---
id: b2c3d4e5-rule-4no-reentry-000000000001
type: rule
status: active
title: No mutex reentrancy in storage helpers
confidence: 0.95
created_at: 2026-08-29T02:50:00Z
updated_at: 2026-08-29T02:50:00Z
---

Any function that internally calls `storage.conn()` (acquiring the
SQLite mutex) must not be called while the mutex is already held.
`std::sync::Mutex` is not reentrant — a second `conn()` call from the
same thread will deadlock.

Before calling any `Storage` method that acquires the mutex internally
(e.g. `db_size_bytes`), drop the current `conn` guard first. If you need
data from the DB and then need to call such a method, fetch the data,
drop the guard, call the method, then re-acquire if needed.
