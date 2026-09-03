---
id: f7a1b2c3-09cc-414e-831c-8f5710374d4a
title: Transaction leak on early return with ? operator
type: knowledge
status: active
created_at: "2026-09-03T11:55:00Z"
updated_at: "2026-09-03T11:55:00Z"
references: []
category: gotchas
tags: ["transaction", "sqlite", "error-handling", "mutex"]
---

# Transaction leak on early return with ? operator

When using raw `BEGIN`/`COMMIT` transactions on the shared SQLite
connection, any `?` early return inside the transaction body leaks
the transaction. The connection stays in transaction mode, breaking
all subsequent DB operations — every query after the leak fails with
"cannot start a transaction within a transaction" or similar.

## The pattern that breaks

```rust
conn.execute_batch("BEGIN")?;
redirect_edges(&conn, ...)?;  // if this fails, transaction leaks
update_entity(&conn, ...)?;   // if this fails, transaction leaks
conn.execute_batch("COMMIT")?;
```

## The fix: closure with explicit ROLLBACK

Wrap the transactional operations in a closure. If the closure
returns an error, execute ROLLBACK before propagating:

```rust
let tx_result: Result<(), Error> = (|| {
    redirect_edges(&conn, ...)?;
    update_entity(&conn, ...)?;
    Ok(())
})();

if let Err(e) = tx_result {
    let _ = conn.execute_batch("ROLLBACK");
    return Err(e);
}
conn.execute_batch("COMMIT")?;
```

## Why not use rusqlite's Transaction guard?

`rusqlite::Transaction` rolls back on drop, which would handle this
automatically. But the shared connection behind `std::sync::Mutex`
makes borrowing the guard across the mutex boundary awkward. The
closure pattern is simpler and equally correct.

## Where this was found

`merge_one` in `src/consolidate/merge.rs` had this bug. The
`redirect_edges`, `update_entity`, and `record_event` calls all used
`?` inside a manually managed transaction. Found in the second full
code audit (2026-09-03).
