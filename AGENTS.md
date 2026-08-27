# CogZ — Agent Guidelines

These are the non-obvious constraints that are easy to violate during
implementation. The nine design documents define *what to build*.
This file defines *what not to break while building it*.

The global workflow directive (plan → implement one subtask → verify →
commit) still applies. This file adds CogZ-specific rules on top.

---

## Before Starting Any Phase

1. **Read the relevant doc section.** Each phase in `implementation-plan.md`
   names what's built and what's NOT built yet. Read it before coding.
2. **Check `dependencies.md`** for pinned versions. Use exact pins for
   pre-release crates (ort). Use caret pins for stable crates. Never
   use floating ranges (`*`, `latest`). Never use versions published
   less than 7 days ago.
3. **Check `testing-strategy.md`** for what tests that phase needs.
   Write tests alongside the implementation, not after.
4. **Check `entity-spec.md`** for frontmatter schema and field types.
   Check `mcp-contract.md` for exact tool signatures and return shapes.

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
- **Code entities** (functions, classes, files, modules) get UUIDs
  assigned by the indexer on first sync. They have no file on disk.

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

## Before Committing Any Phase

- [ ] `cargo build --release` succeeds
- [ ] `cargo test` passes (default, no models)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] No integer entity IDs introduced (grep for `i64` in new code)
- [ ] No direct DB writes outside `storage/` (file-first invariant)
- [ ] No new dependencies without checking `dependencies.md` and the
      7-day rule
- [ ] Tests from `testing-strategy.md` for this phase are written and
      passing
- [ ] Rebuildability check: `cogz reset` + `cogz index` still works
      (if phase touches storage or file sync)

---

## Document References

| Document | What it defines |
|---|---|
| `goal.md` | What CogZ is and isn't |
| `first-principles.md` | Axiomatic design principles |
| `architecture.md` | Schema, modules, concurrency, search, context, retention |
| `entity-spec.md` | File format, frontmatter schema, update policy, state machine |
| `mcp-contract.md` | 13 MCP tool signatures and return shapes |
| `implementation-plan.md` | 12 phases, what's built, verification criteria |
| `testing-strategy.md` | Test categories, fixtures, mock model, coverage targets |
| `dependencies.md` | Pinned crate versions, build decisions, Cargo.toml |
| `packaging.md` | Distribution, install, update, uninstall, versioning |
