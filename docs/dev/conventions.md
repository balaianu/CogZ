# Conventions

This document describes the code patterns and invariants that govern CogZ. New code must follow these patterns.

## File-first invariant

Every write writes the file first, then syncs the DB. If the file write fails, the DB is not updated. Sync is one-directional: file → DB. The DB is disposable — `cogz reset` + `cogz index` rebuilds everything from files and source code.

No tool, no CLI command, no consolidation step writes directly to the DB outside `src/storage/`. The storage layer is the only code that touches SQLite directly.

## UUIDs everywhere

Entity IDs are UUID strings everywhere — in file frontmatter, in the DB primary key, in MCP tool parameters and returns, in `references` fields. No integer entity IDs.

- **File-backed entities** (observation, rule, knowledge) use UUID v4.
- **Code entities** (function, class, file, module) use deterministic UUID v5 derived from `{file_path}:{entity_type}:{qualified_name}`. The same code entity gets the same UUID across rebuilds, preserving `references` edges from observations.

## Concurrency

- **Single SQLite `Connection` behind `std::sync::Mutex`.** No connection pool, no `r2d2`, no `deadpool`, no `tokio-rusqlite`.
- **DB calls from async MCP handlers go through `tokio::task::spawn_blocking`.** Never call rusqlite directly from an async context.
- **Don't hold the mutex during filesystem or network I/O.** Acquire the lock, get the data, drop the lock, then do the I/O. The `db_size_bytes` function is the reference pattern: it queries `PRAGMA database_list` under the lock, extracts the path, drops the guard via a block scope, then calls `std::fs::metadata` outside the lock.

## Query batching

When processing a collection of items that each need a SQL query, batch them into a single query using `IN (?, ?, ...)` instead of one query per item. The `get_neighbors_batch` function is the reference pattern: instead of calling `get_edges_from` per frontier node (N queries), it fetches all neighbors for all frontier nodes in 2 queries.

Build placeholders dynamically:
```rust
let placeholders = (0..n).map(|_| "?").collect::<Vec<_>>().join(",");
let params: Vec<&dyn rusqlite::ToSql> = items.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
```

## Read once, pass forward

When a file is read during sync, the parsed result should be returned to the caller so it doesn't need to be re-read. The `sync_one_file_inner` function is the reference pattern: it returns `(SyncAction, EntityFile)` so callers can use the entity ID without reading the file again.

## Typed errors

Library modules use `thiserror` enums with `#[from]` for automatic conversion. The CLI binary (`main.rs`) uses `anyhow` for top-level error handling.

- Never use `String` as an error type.
- Error variants should be specific: `StorageError::EntityNotFound` is useful; `StorageError::Generic("entity not found")` is not.
- `#[from]` ambiguity: if an enum has `#[from]` for `StorageError` and `StorageError` itself has `#[from]` for `rusqlite::Error`, you cannot also add `#[from]` for `rusqlite::Error` to the outer enum. Convert manually with `.map_err(|e| OuterError::Storage(e.into()))`.

## Logging

- **Library code:** `tracing` macros (`warn!`, `debug!`, `info!`). Never `println!` in library code — library modules don't know if they're being called from a CLI, an MCP server, or a test.
- **CLI code:** `println!` for user-facing output.
- `tracing::warn!` for recoverable failures (model unavailable, embedding failed for one entity).
- `tracing::debug!` for diagnostic detail that's noisy by default.

## Append-only policy

- **No `update_observation` tool.** No `edit_rule` tool. Observations are append-only for content. Rules are append-only for substantive changes (supersede + new rule).
- **`update_knowledge` is the only content-edit tool.** It overwrites the knowledge file and re-syncs the DB.
- **Status changes are in-place** (frontmatter only, body untouched). These go through `transition_status()` — illegal transitions fail with an error.

## Graceful degradation

- The system must function without ONNX models. No model download is required for `cogz init`, `cogz index` (files sync without embeddings), or `cogz search` (FTS-only).
- Never panic on model unavailability. Log a warning, set `available: false` in status, continue with degraded capability.
- `clean_broken_cache()` runs before every model load: removes `.incomplete` files >1h old and empty `refs/main` files. Never panic on cleanup failures — log and continue.

## Token counting

Use the chars/4 heuristic: `tokens ≈ len(text) / 4`. No external tokenizer dependency for context pack budgeting. The function lives in `context/compress.rs` behind a trait so it can be swapped later.

## Retention

- Only `rejected` and `superseded` observations are prunable. Active and stale are never pruned. Rules and knowledge are never pruned.
- Pruning is never automatic. Requires explicit `cogz doctor --prune-observations --confirm`. Default is dry-run.
- Pruned entities become tombstones: `status='pruned'`, no content/embedding/FTS, graph edges preserved. Terminal state.

## File size

400 lines max per file. If a file approaches this, extract self-contained functions to a new module in the same directory. The `files/refs.rs` extraction from `files/sync.rs` is the reference pattern.

## Comments

Comments explain **why**, never **what**. The code already says what.

- **Module doc comments (`//!`):** 1–3 sentences. What the module is for. One non-obvious constraint if relevant. No architecture walkthroughs.
- **Item doc comments (`///`):** First line: what it does, imperative mood. Include `# Arguments` / `# Returns` / `# Errors` only when the signature is non-obvious or it's public API.
- **Inline comments:** Only for **why**. Never for **what**.
- **Test comments:** Don't restate the test function name. Only add one if the test has non-obvious setup or a subtle assertion.

## Pre-commit verification

Before committing any change:

```bash
cargo build --release
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

Additionally:
- No integer entity IDs in new code (grep for `i64` in entity ID contexts).
- No direct DB writes outside `src/storage/`.
- No new dependencies without checking `dependencies.md` and the 7-day rule.
- Rebuildability check: `cogz reset` + `cogz index` still works if the change touches storage or file sync.

## See also

- [Dependencies](dependencies.md) — pinned crate versions
- [Testing](testing.md) — test categories and mock models
- [Schema](schema.md) — DB schema and migrations
