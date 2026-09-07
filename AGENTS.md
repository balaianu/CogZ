# CogZ — Agent Guidelines

These are the non-obvious constraints that are easy to violate during
implementation. The design documents define *what to build*.
This file defines *what not to break while building it*.

The global workflow directive (plan → implement one subtask → verify →
commit) still applies. This file adds CogZ-specific rules on top.

---

## Before Starting Any Phase

1. **Read the relevant design doc.** `docs/design/architecture.md`
   covers the system overview and module map. Read it before coding.
2. **Check `docs/dev/dependencies.md`** for pinned versions. Use exact pins for
   pre-release crates (ort). Use caret pins for stable crates. Never
   use floating ranges (`*`, `latest`). Never use versions published
   less than 7 days ago.
3. **Check `docs/dev/testing.md`** for what tests that phase needs.
   Write tests alongside the implementation, not after.
4. **Check `docs/design/entity-model.md`** for frontmatter schema and field types.
   Check `docs/integration/mcp-tools.md` for exact tool signatures and return shapes.

---

## Hard Constraints

These are architectural invariants. Violating them breaks the system's
core promises. The compiler won't catch most of them.

### Files are canonical, DB is derived

- **Every write writes the file first, then syncs the DB.** No tool,
  no CLI command, no consolidation step writes directly to the DB.
  If the file write fails, the DB is not updated.
- **Sync is one-directional: file → DB.** No bidirectional sync, no
  conflict resolution, no "which is newer?" logic.
- **The DB is disposable.** `cogz reset` + `cogz index` must rebuild
  everything from files + source code. If a change breaks
  rebuildability, it's wrong.

### UUIDs everywhere

- **Entity IDs are UUID v4 strings.** In file frontmatter, in the DB
  primary key, in MCP tool parameters and returns, in `references`
  fields. No integer entity IDs anywhere.
- **`references`, `supporting_ids`, `promoted_from`, `superseded_by`**
  all use UUID strings, not integers. This is what makes files
  self-contained and portable across DB rebuilds.
- **Code entities** (functions, classes, files, modules) get
  deterministic UUID v5 values derived from
  `{file_path}:{entity_type}:{qualified_name}`. They have no file
  on disk. The same code entity gets the same UUID across rebuilds.

### Concurrency

- **Single SQLite `Connection` behind `std::sync::Mutex`.** No
  connection pool, no `r2d2`, no `deadpool`, no `tokio-rusqlite`.
- **DB calls from async MCP handlers go through
  `tokio::task::spawn_blocking`.** Never call rusqlite directly from
  an async context — it blocks the tokio runtime.

### Append-only policy

- **No `update_observation` tool.** No `edit_rule` tool. These do not
  exist and must not be added. Observations are append-only for
  content. Rules are append-only for substantive changes (supersede
  + new rule).
- **`update_knowledge` is the only content-edit tool.** It overwrites
  the knowledge file and re-syncs the DB.
- **Status changes are in-place** (frontmatter only, body untouched).
  These go through `transition_status()` — illegal transitions must
  fail with an error.

### Graceful degradation

- **The system must function without ONNX models.** No model download
  should be required for `cogz init`, `cogz index` (files sync without
  embeddings), or `cogz search` (FTS-only).
- **Auto-download is opt-out, not opt-in.** When `auto_download = true`
  (default), `cogz index` fetches models from HuggingFace via `hf-hub`.
  `--no-download` or `auto_download = false` disables this. The system
  must still function in FTS-only mode when download is disabled or
  fails.
- **Partial download cleanup.** `clean_broken_cache()` runs before
  every model load: removes `.incomplete` files >1h old and empty
  `refs/main` files. Prevents disk-filling retry loops on failed
  downloads. Never panic on cleanup failures — log and continue.
- **Title-based dedup works without embeddings.** Exact + fuzzy title
  match does not require the embedding model. Embedding similarity
  dedup degrades to title-only when models are absent.
- **NLI unavailable → no contradiction detection.** This is
  acceptable. The system continues to function.
- **Never panic on model unavailability.** Log a warning, set
  `available: false` in status, continue with degraded capability.

### Token counting

- **Use the chars/4 heuristic.** `tokens ≈ len(text) / 4`. No
  external tokenizer dependency for context pack budgeting. The
  function lives in `context/compress.rs` behind a trait so it can
  be swapped later if needed.

### Retention

- **Only `rejected` and `superseded` observations are prunable.**
  Active and stale are never pruned. Rules and knowledge are never
  pruned.
- **Pruning is never automatic.** Requires explicit `cogz doctor
  --prune-observations --confirm`. Default is dry-run.
- **Pruned entities become tombstones.** Minimal DB record, no
  content/embedding/FTS, graph edges preserved. `status='pruned'` is
  terminal.

---

## Code Comments and Doc Comments

Comments explain **why**, never **what**. The code already says what.
If a comment restates the line below it, delete the comment.

### Module doc comments (`//!`)

1–3 sentences. What the module is for. One non-obvious constraint if
relevant. No architecture walkthroughs, no "how it works", no
citations to design docs. The code stands on its own.

### Item doc comments (`///`)

First line: what the function/struct/enum does, imperative mood.
Include `# Arguments` / `# Returns` / `# Errors` only when the
signature is non-obvious or it's public API. For trait methods with
default implementations, a one-liner is sufficient.

### Inline comments

Only for **why**. Never for **what**. Use them to explain non-obvious
decisions, bug-prevention context, or subtle ordering constraints.
Section dividers (`// ── Section ───`) are fine in long files for
navigation.

### Test comments

Don't restate the test function name. Most tests don't need a
comment. Only add one if the test has non-obvious setup, a subtle
assertion, or an edge case not clear from the name.

### Tone

Neutral, imperative. No "per architecture.md section X", no "per
entity-model". Say what something IS, not what it isn't. No defensive
disclaimers.

---

## Testing Philosophy

`docs/dev/testing.md` defines *what* to test per phase. This section
defines *how to think about* testing.

### Test when

- There is a non-obvious correctness invariant that could break
  silently (content-hash skip, status lifecycle, stale marking)
- There is branching logic where one branch is rarely exercised
  (error paths, fallback logic, edge cases)
- It is a public contract that other code depends on (sync semantics,
  status transitions, edge integrity)
- It involves SQL, concurrency, state transitions, or external I/O
  error handling

### Don't test when

- The code is a straight-line assignment or return with no branches
- The test would only fail if Rust itself were broken (`String::len`,
  `Vec::push`, `HashMap::insert`)
- The test is a duplicate with trivially different input hitting the
  same code path — use a loop over cases in one test instead
- The test requires external resources that can't be meaningfully
  tested without production infrastructure

### How to test

- **Prefer integration over isolation.** A test that chains
  file-read → sync → DB-query and verifies DB state catches more than
  three unit tests testing each in isolation. This aligns with
  `docs/dev/testing.md`: real SQLite, real filesystem.
- **Loop over cases, don't duplicate.** One test function with a
  `for (input, expected) in cases` loop beats N copy-pasted test
  functions.
- **Verify output state, not implementation details.** Assert what
  the result is (DB rows, return values, file contents), not which
  internal methods were called or in what order.
- **Test the contract, not the wrapper.** A function that delegates
  to a `HashMap` doesn't need its own test. The code that depends on
  the contract does.
- **Every test must have a failure mode.** If you can't describe what
  bug the test would catch, delete it.

### The ratio test

If you removed a test, would you lose confidence in the correctness
of the system? If no, the test is there for coverage, not for value.
Delete it.

---

## Codebase Patterns

These patterns are established in the current codebase and must be
followed in all new code. They exist because they prevent real bugs
or performance problems that have been identified during review.

### Typed errors, no String error types

Library modules use `thiserror` enums with `#[from]` for automatic
conversion. The CLI binary (`main.rs`) uses `anyhow` for top-level
error handling.

- **Never use `String` as an error type.** If a function can fail in
  multiple ways, define an error enum. If it delegates to another
  module's error, use `#[from]` or `.map_err()`.
- **Error variants should be specific.** `StorageError::EntityNotFound`
  is useful; `StorageError::Generic("entity not found")` is not.
- **`#[from]` ambiguity:** if an enum has `#[from]` for `StorageError`
  and `StorageError` itself has `#[from]` for `rusqlite::Error`, you
  cannot also add `#[from]` for `rusqlite::Error` to the outer enum.
  Convert manually with `.map_err(|e| OuterError::Storage(e.into()))`.

### Mutex and I/O

The single SQLite `Connection` is behind `std::sync::Mutex`. Any
function that needs the connection acquires it via `storage.conn()`.

- **Don't hold the mutex during filesystem or network I/O.** Acquire
  the lock, get the data you need (e.g. a file path), drop the lock,
  then do the I/O. Holding the lock during I/O blocks all other
  callers and can deadlock.
- **The `db_size_bytes` function is the reference pattern:** it
  queries `PRAGMA database_list` under the lock, extracts the path,
  drops the guard via a block scope, then calls `std::fs::metadata`
  outside the lock.

### Query batching

When processing a collection of items that each need a SQL query,
batch them into a single query using `IN (?, ?, ...)` instead of
one query per item.

- **The `get_neighbors_batch` function is the reference pattern:**
  instead of calling `get_edges_from` per frontier node (N queries),
  it fetches all neighbors for all frontier nodes in 2 queries
  (one outgoing, one incoming).
- **Build placeholders dynamically:** `(0..n).map(|_| "?").join(",")`
  and bind params via `Vec<&dyn rusqlite::ToSql>`.
- **Apply this proactively** to any new code that loops over a
  collection and queries the DB per item.

### Read once, pass forward

When a file is read during sync, the parsed result should be returned
to the caller so it doesn't need to be re-read.

- **The `sync_one_file_inner` function is the reference pattern:** it
  returns `(SyncAction, EntityFile)` so callers can use the entity ID
  for `seen_ids` and `synced_entity_ids` without reading the file
  again.
- **Applies to any I/O:** if a function reads a file or queries the
  DB, return the result. Don't make callers repeat the work.

### Logging

Library code uses `tracing` macros (`warn!`, `debug!`, `info!`).
CLI code uses `println!` for user-facing output.

- **`tracing::warn!`** for recoverable failures (model unavailable,
  embedding failed for one entity).
- **`tracing::debug!`** for diagnostic detail that's noisy by default
  (cache hit/miss, sync skip reasons).
- **Never use `println!` in library code.** Library modules don't
  know if they're being called from a CLI, an MCP server, or a test.

### File size

- **400 lines max per file.** If a file approaches this, extract
  self-contained functions to a new module in the same directory.
  The `files/refs.rs` extraction from `files/sync.rs` is the
  reference pattern.

---

## Before Committing Any Phase

- [ ] `cargo build --release` succeeds
- [ ] `cargo test` passes (default, no models)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] No integer entity IDs introduced (grep for `i64` in new code)
- [ ] No direct DB writes outside `storage/` (file-first invariant)
- [ ] No new dependencies without checking `docs/dev/dependencies.md` and the
      7-day rule
- [ ] Tests from `docs/dev/testing.md` for this phase are written and
      passing
- [ ] Rebuildability check: `cogz reset` + `cogz index` still works
      (if phase touches storage or file sync)

---

## Document References

| Document | What it defines |
|---|---|
| `docs/getting-started.md` | Mental model and walkthrough |
| `docs/configuration.md` | Full `config.toml` reference |
| `docs/cli-reference.md` | Every command and flag |
| `docs/integration/mcp-tools.md` | 13 MCP tool signatures and return shapes |
| `docs/integration/hooks.md` | Lifecycle events and output format |
| `docs/integration/agent-setup.md` | Devin, Claude Code, generic MCP setup |
| `docs/design/architecture.md` | System overview, module map, data flow |
| `docs/design/entity-model.md` | File format, frontmatter schema, update policy, state machine |
| `docs/design/search.md` | Hybrid FTS + vector, RRF, graph expansion |
| `docs/design/consolidation.md` | Dedup, contradiction, promotion, merge |
| `docs/design/degradation.md` | FTS-only mode and fallback behavior |
| `docs/dev/building.md` | Build, release, cross-compile |
| `docs/dev/testing.md` | Test categories, fixtures, mock model, coverage targets |
| `docs/dev/conventions.md` | Code patterns and invariants |
| `docs/dev/dependencies.md` | Pinned crate versions, build decisions, Cargo.toml |
| `docs/dev/schema.md` | DB schema and migrations |
