---
id: 6fd6f5fe-d983-4c9c-be26-405606749540
type: knowledge
status: active
title: get_status Mutex Deadlock
category: gotchas
tags: [mcp, deadlock, mutex, storage]
created_at: 2026-08-29T02:35:00Z
updated_at: 2026-08-29T02:35:00Z
---

# get_status Mutex Deadlock

The `get_status` MCP tool deadlocks if it calls `storage.db_size_bytes()`
while already holding the storage mutex via `storage.conn()`.

## Root cause

`Storage::db_size_bytes()` internally calls `self.conn()` to acquire the
mutex, query `PRAGMA database_list` for the DB file path, then drop the
lock before doing `std::fs::metadata`. But if the caller already holds
the mutex (via a live `conn` guard), the second `conn()` call blocks
forever — `std::sync::Mutex` is not reentrant.

## Fix

Get the DB path under a short-lived lock, drop it, do the filesystem
I/O, then re-acquire the lock for the remaining queries:

```rust
let db_path = {
    let conn = storage.conn();
    conn.query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2)).ok()
};
let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
let conn = storage.conn(); // re-acquire for queries
```

## General lesson

Any function that internally acquires the storage mutex (like
`db_size_bytes`) must not be called while the mutex is already held.
This is the same pattern documented in the "Mutex and I/O" section of
AGENTS.md, but it applies to *any* mutex-acquiring helper, not just
filesystem I/O.
