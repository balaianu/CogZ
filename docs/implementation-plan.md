# CogZ — Implementation Plan

This document defines the build order, phases, and verification
criteria. Each phase produces something independently testable. No
phase depends on a later phase to be verified.

---

## Design Principles for the Build

1. **Bottom-up.** Storage before search. Search before context.
   Context before MCP. Each layer builds on the one below it.
2. **Testable at every step.** Each phase has a verification that
   proves it works without needing the next phase.
3. **MVP first, then depth.** Get a minimal end-to-end pipeline
   working (file → DB → search → context), then add indexing,
   consolidation, code-awareness, and hooks.
4. **No speculative code.** If a phase doesn't need it, don't build
   it. The architecture doc defines what exists; this plan defines
   when.

---

## Phase 1: Project Skeleton + Config

**Goal:** A compilable Rust project with config loading and validation.

**What's built:**
- `Cargo.toml` with all dependencies (per dependencies.md)
- `src/main.rs` with clap CLI skeleton (`cogz init`, `cogz --version`)
- `src/config/` — typed config structs, TOML loading, validation
- `cogz init` — creates `.cogz/` directory structure, generates
  `config.toml` with defaults, autodetects project name from repo
  directory or git remote, generates `.gitignore`

**Verification:**
- `cargo build --release` succeeds
- `cogz --version` prints version
- `cogz init` in a test repo creates correct directory structure
- Generated `config.toml` is valid and parseable
- Generated `.gitignore` ignores observations/ and cogz.db
- Project name is correctly autodetected

**What's NOT built yet:** Storage, indexing, search, MCP — nothing
else. Just the skeleton and config.

---

## Phase 2: Storage Layer

**Goal:** A working SQLite database with the full schema, CRUD
operations, and query functions.

**What's built:**
- `src/storage/schema.rs` — table definitions, migrations, `PRAGMA user_version`
- `src/storage/crud.rs` — insert, update, get, delete entities;
  insert/query edges
- `src/storage/query.rs` — read queries (by type, by status, by
  file_path, graph traversal)
- `src/storage/events.rs` — domain event recording
- `src/storage/status.rs` — status state machine (`transition_status()`,
  validates all legal/illegal transitions)
- SQLite WAL mode, FTS5, vec0 initialization

**Verification:**
- Unit tests: insert an entity, query it back, verify all fields
- Unit tests: insert edges, traverse the graph (1-hop, 2-hop)
- Unit tests: FTS5 search returns correct results
- Unit tests: vec0 insert + similarity search returns correct results
- Unit tests: events are recorded on insert
- Unit tests: status state machine — all legal transitions succeed,
  all illegal transitions fail (rejected→active, superseded→active)
- `cogz status` prints DB stats (entity counts by type, DB size)
- Schema version is stored and checked on open

**What's NOT built yet:** File sync, indexing, search fusion, context.
The storage layer is tested via direct API calls, not through files
or MCP.

---

## Phase 3: File Layer

**Goal:** File → DB sync works for all entity types. Files are
canonical; DB is derived.

**What's built:**
- `src/files/entities.rs` — markdown file I/O, YAML frontmatter
  parsing, entity file writing
- `src/files/sync.rs` — scan `.cogz/` directories, detect changes
  (content hash), sync to DB
- `cogz index` (partial) — scans `.cogz/knowledge/`, `.cogz/rules/`,
  `.cogz/observations/`, syncs to DB
- `cogz reindex` — incremental reindex (only changed files)
- `cogz reset` — drops DB
- `cogz reset --purge` — drops DB + observations + generated .gitignore

**Verification:**
- Create a knowledge file manually → `cogz index` → entity in DB
- Edit the file → `cogz reindex` → DB entity updated
- Delete the file → `cogz reindex` → DB entity marked stale
- Create an observation file → indexed despite being gitignored
- Content hash correctly detects changes (no unnecessary re-embedding)
- `cogz reset` drops DB, `cogz index` rebuilds from files
- `cogz reset --purge` removes observations but NOT knowledge/rules

**What's NOT built yet:** Code indexing (tree-sitter), embeddings,
search, MCP. Files are synced to DB but without embeddings or FTS
population (FTS works but vec0 is empty).

---

## Phase 4: Embedding Layer

**Goal:** ONNX models load, embed text, and cache results.

**What's built:**
- `src/embed/model.rs` — `EmbeddingModel` trait
- `src/embed/onnx.rs` — ONNX Runtime implementation (CodeRankEmbed,
  bge-base)
- `src/embed/cache.rs` — content-hash-based embedding cache
- Model download on first use (HuggingFace via `hf-hub`, cached at
  `~/.local/share/cogz/models/`). Auto-download unless
  `[embedding].auto_download = false` or `--no-download` flag.
  See Phase 12 for `src/embed/download.rs` (download + cleanup).
- Integration with file sync: after syncing a file, embed its content
  and store in vec0

**Verification:**
- Model downloads on first `cogz index` (if not cached and auto_download enabled)
- `cogz index --no-download` succeeds in FTS-only mode
- Embedding produces correct-dimension vectors
- Cache prevents re-embedding unchanged content
- If model unavailable: `cogz index` succeeds, entities stored
  without vectors (graceful degradation)
- `cogz status` reports model availability and cache size

**What's NOT built yet:** Search (vectors are stored but not
queried), NLI, reranker. Just embedding + storage.

---

## Phase 5: Search

**Goal:** Hybrid FTS5 + vector search with RRF fusion and graph
expansion.

**What's built:**
- `src/search/hybrid.rs` — run FTS5 and vec0 searches, merge
- `src/search/rrf.rs` — reciprocal rank fusion
- `src/search/expand.rs` — graph-aware expansion from search results
  (follow edges, collect related entities, record graph paths)
- `cogz search <query>` — CLI command, prints ranked results

**Verification:**
- FTS-only search works (no model needed)
- Hybrid search works (FTS + vec, RRF fusion)
- Graph expansion follows edges and returns related entities
- Graph paths are recorded (provenance for each result)
- Results are ranked correctly (RRF produces sensible ordering)
- Search filters by entity type (code vs knowledge)
- `cogz search "FTS5 ranking"` returns relevant entities

**What's NOT built yet:** Context assembly, MCP. Search is a CLI
command that prints results.

---

## Phase 6: Context Assembly

**Goal:** Context packs are assembled from search results with
provenance and token budgeting.

**What's built:**
- `src/context/modes.rs` — cold_start, task, escalation mode definitions
- `src/context/assemble.rs` — build ContextPack from search results +
  graph expansion, include graph paths
- `src/context/compress.rs` — token budget management, section
  prioritization, truncation
- `cogz context [--mode] [query]` — CLI command, prints context pack

**Verification:**
- `cold_start` mode produces a compact pack (repo identity, recent
  rules, recent observations)
- `task` mode produces a query-scoped pack with graph paths
- `escalation` mode produces a wider pack (more results, more hops)
- Token budget is respected (pack size ≤ configured limit)
- Graph paths are included for each section
- Dropped sources are listed in metadata
- `cogz context --mode task "FTS5 ranking"` produces a coherent pack

**What's NOT built yet:** MCP server, hooks. Context is a CLI
command.

---

## Phase 7: MCP Server

**Goal:** MCP server exposes all tools, agent can interact with CogZ.

**What's built:**
- `src/mcp/server.rs` — MCP protocol server (stdio), `ServerHandler` impl
- `src/mcp/tools.rs` — 11 tool handlers (Phase 7 scope; `consolidate`
  deferred to Phase 9, `capture_event` to Phase 11)
- `src/mcp/params.rs` — parameter structs (`serde::Deserialize` +
  `schemars::JsonSchema`)
- `src/mcp/helpers.rs` — file-first write logic, response builders,
  error helpers, embedding helper
- `src/mcp/dedup.rs` — title match (exact + fuzzy) + embedding
  similarity dedup
- `cogz mcp-stdio` — runs the MCP server over stdio transport
- Integration with all layers: storage, files, search, context
- Concurrency: single SQLite `Connection` behind `Mutex`, DB calls
  via `tokio::task::spawn_blocking` from async MCP handlers
- `update_knowledge` tool — the only content-edit tool (knowledge
  in-place updates; observations and rules are append-only)
- Basic dedup in insert path: title match (exact + fuzzy) + embedding
  similarity > threshold → `duplicate_warning` returned to caller.
  This is lightweight, synchronous, and does not require NLI.
- Graceful degradation: missing embedding models → FTS-only search,
  title-only dedup
- 16 integration tests via `tokio::io::duplex` transport

**Tools implemented (11):**
1. `record_observation` 2. `query_observations` 3. `create_rule`
4. `query_rules` 5. `create_knowledge` 6. `update_knowledge`
7. `query_knowledge` 8. `search` 9. `get_context` 10. `get_status`
11. `list_entities`

**Verification:**
- MCP server starts and responds to tool list request (11 tools)
- Each tool works correctly (record_observation creates file + DB
  entry, search returns results, get_context returns a pack,
  update_knowledge edits a knowledge file in-place)
- Duplicate detection: create observation with same title as existing
  → `duplicate_warning` returned
- Error handling: invalid parameters return proper MCP errors
- `cogz mcp-stdio` works when configured in an agent's MCP config
- Rebuildability: files written by MCP tools can be re-synced to a
  fresh DB via `cogz index`

**What's NOT built yet:** Code indexing, full consolidation
(contradiction detection, promotion, merge), hooks. MCP tools work
for file-based entities but code entities don't exist yet. Basic
dedup (title + embedding similarity) is built here; full consolidation
is Phase 9. `consolidate` and `capture_event` tools deferred.

---

## Phase 8: Code Indexing ✅

**Goal:** Tree-sitter parses source code, code entities and
structural edges are inserted into the graph.

**What's built:**
- `src/index/gitignore.rs` — gitignore-aware source file scanning
  using the `ignore` crate, with `[index].allow` override via
  `globset`. Two-pass scan: standard gitignore traversal + explicit
  allow-list unfiltered traversal.
- `src/index/tree_sitter.rs` — AST parsing for Rust and Python.
  Extracts functions, classes (structs/impls/enums/traits for Rust,
  classes for Python), files, and modules with properties
  (line_start, line_end, signature, qualified_name, language).
- `src/index/sync/mod.rs` — code entity synchronization with
  deterministic UUID v5 IDs (`{file_path}:{entity_type}:{qualified_name}`).
  Content hash change detection, insert/update/stale/reactivate,
  preserves `created_at` on updates.
- `src/index/code_graph/mod.rs` — structural edge extraction (calls,
  imports, extends) from tree-sitter AST. Name-to-UUID matching
  with both qualified and simple name lookup.
- `src/index/mod.rs` — orchestration: scan → read → parse → sync
  entities → sync edges. Integrated into `cogz index` and
  `cogz reindex` CLI commands.
- Multi-model embedding (CodeRankEmbed for code, bge-base for
  knowledge) — already implemented in `cli.rs` from Phase 7.
- `OnnxEmbeddingModel::with_model_id()` — wires `code_model` and
  `knowledge_model` config fields to actual model directory paths.
  Updated in `cli.rs`, `mcp/server.rs`, and `mcp/status.rs`.
- True batched ONNX inference — pad/collate input_ids into a single
  `[batch, max_seq_len]` tensor, one `session.run()` call per batch.
- Fused multi-seed BFS for graph expansion — single shared frontier
  across all seeds, one `get_edges_involving_batch` query per hop.
- Chunked `IN (...)` queries — all batch query functions chunk at
  `SQLITE_MAX_VARIABLE_NUMBER / params_per_row` to avoid SQLite
  variable limits. Affected: `get_edges_involving_batch` (49),
  `batch_check_status` (998), `get_references_batch` (999),
  `get_neighbors_batch` (999), `get_entities_batch` (999).

**Verification:**
- ✅ Index a real Rust repo → functions, classes, files appear as entities
- ✅ Index Python source → functions, classes, files appear as entities
- ✅ Structural edges (calls, imports, extends) are correct
- ✅ Gitignored files are NOT indexed
- ✅ `allow` config overrides gitignore for specific paths
- ✅ Code entities use Code model, knowledge uses Knowledge model
- ✅ Search returns both code and knowledge results
- ✅ Rebuildability: `cogz reset` + `cogz index` produces identical UUIDs
- ✅ Stale marking: removed source files → entities marked stale
- ✅ 279 tests pass (234 baseline + 45 new)

**What's NOT built yet:** Git diff stale flagging, consolidation,
hooks. Code is indexed but change detection is manual (full reindex).

---

## Phase 9: Consolidation

**Goal:** Full consolidation — contradiction detection, promotion, and
merge — built on top of the basic dedup from Phase 7.

**What's built:**
- `src/consolidate/contradict.rs` — NLI model check on insert
- `src/consolidate/promote.rs` — observation → rule promotion
  (background, threshold-based)
- `src/consolidate/merge.rs` — duplicate merging, edge redirection
- `src/embed/model.rs` — `NliModel` trait + ONNX implementation
- `cogz consolidate` — trigger background consolidation manually
- `consolidate` MCP tool — 12th MCP tool, runs promotion + merge
  (brings total from 11 to 12 tools; `capture_event` remains Phase 11)
- Full dedup module (`src/consolidate/dedup.rs`) promoted from Phase 7
  basic check: now includes consolidation-driven merge decisions
- Integration with insert path: every insert triggers dedup (Phase 7)
  + contradict (Phase 9)

**Verification:**
- Insert a duplicate observation → flagged as potential duplicate
- Insert a knowledge entry with same title as existing → `duplicate_warning` returned
- Insert a contradicting rule → contradiction detected and flagged
- Observations with enough support → promoted to rules
- Merged entities: survivor keeps edges, superseded entity marked
- `cogz consolidate` runs background promotion + merge
- NLI model unavailable: contradiction check skipped (graceful
  degradation), dedup still works (embedding-based)

**What's NOT built yet:** Hooks.

---

## Phase 10: Git Diff + Stale Flagging

**Goal:** Code changes are detected, affected knowledge is flagged.

**What's built:**
- `src/index/git_diff.rs` — detect modified/deleted code entities
  via git diff (baseline tree → working directory, includes uncommitted)
- `src/index/stale_flagging.rs` — mark observations/rules/knowledge
  referencing changed code as `status = 'stale'` (file-first: updates
  frontmatter on disk before DB)
- `code_changed` domain event
- `cogz reindex` uses `reindex_code()` — incremental, only re-parses
  changed files; falls back to full scan when no git baseline exists
- `src/index/code_graph/incremental.rs` — incremental structural edge
  sync: deletes only edges from changed-file entities, rebuilds from
  changed files, preserves edges from unchanged files. Builds name-to-UUID
  map from all DB entities for cross-file target resolution.
- `src/storage/edges.rs` — `delete_structural_edges_by_sources` for
  scoped edge deletion (only changed source entities' outgoing edges)
- Baseline commit SHA stored in meta table as `last_indexed_commit`

**Verification:**
- Modify a source file → `cogz reindex` → code entity updated
- Observation referencing that function → marked stale
- `code_changed` event recorded
- Unchanged files are not re-parsed (incremental, fast)
- `cogz status` reports stale entity count
- Deleted source file → code entities stale + referenced knowledge stale
- Non-git repo → falls back to full scan
- Incremental reindex preserves structural edges from unchanged files
  (regression: full index → modify one file → reindex → edge count unchanged)

**What's NOT built yet:** Hooks. Everything else is done.

---

## Phase 11: Hooks

**Goal:** Lifecycle events are captured, context is injected.

**What's built:**
- `src/hooks/lifecycle.rs` — session_start, prompt_submit,
  pre_tool_use, post_tool_use handlers
- `src/hooks/capture.rs` — `cogz capture-event` CLI command
- session_start → generates cold_start context pack, prints to stdout
- prompt_submit → generates task context pack, prints to stdout
- pre/post_tool_use → records observations if meaningful

**Verification:**
- `cogz capture-event session_start` prints a cold_start context pack
- `cogz capture-event prompt_submit --prompt "search bug"` prints a
  task context pack
- Events are recorded in the DB
- Hook scripts work when configured in an agent's hooks config

---

## Phase 12: Polish + Release Prep

**Goal:** Production-ready binary, documentation, install script.

**What's built:**
- `src/embed/download.rs` — model download via `hf-hub`:
  - `download_model(model_id)` — fetch model.onnx + tokenizer.json
  - `clean_broken_cache()` — remove `.incomplete` files >1h old,
    remove empty `refs/main` files. Runs before every model load.
    Prevents the disk-filling retry loop that affected Mnemos.
  - `auto_download` config option (default: true)
  - `--no-download` CLI flag on `cogz index`
- `cogz models` — model management:
  - `cogz models download` — download all configured models
  - `cogz models download --code|--knowledge|--nli` — download specific model
  - `cogz models list` — show configured models and download status
  - `cogz models clean` — run `clean_broken_cache()` manually
- `cogz update` — self-update from GitHub releases
- `cogz doctor` — health check with policy violation detection:
  - DB integrity, model availability, file sync consistency
  - Observation content edits (policy violation — observations are append-only)
  - Rule content edits (substantive changes logged)
  - Illegal status transitions (rejected → active without new entity)
  - Orphaned supersedes (rule superseded but no derived_from edge)
  - Missing files (DB entity exists but file gone, not marked stale)
  - Near-duplicate knowledge entries (pairwise similarity > 0.80)
- `cogz doctor --prune-observations` — retention:
  - Dry-run (default): report rejected/superseded observations older
    than `observation_prune_after_days`, grouped by age
  - `--confirm`: delete observation files, replace DB entities with
    tombstones (minimal record preserving graph edges), cap at
    `tombstone_max_count`
  - Active and stale observations are never pruned
  - Rules and knowledge are never pruned
- Install script (`install.sh`)
- README.md
- GitHub release workflow (CI)
- Performance profiling and optimization (memory, speed)

**Verification:**
- `cogz models download` fetches all models to `~/.local/share/cogz/models/`
- `cogz models list` shows download status for each model
- `cogz index` auto-downloads models when `auto_download = true`
- `cogz index --no-download` skips download, operates in FTS-only mode
- `[embedding].auto_download = false` disables auto-download
- `clean_broken_cache()` removes `.incomplete` files older than 1 hour
- `clean_broken_cache()` removes empty `refs/main` files
- `clean_broken_cache()` preserves `.incomplete` files younger than 1 hour
- Interrupted download + retry does not fill disk (cleanup runs before retry)
- `cogz models clean` manually runs cleanup and reports what was removed
- `cogz doctor` reports all healthy on a freshly indexed repo
- `cogz doctor` detects and reports policy violations (edited
  observation, illegal status transition, orphaned supersede)
- `cogz doctor` detects near-duplicate knowledge entries (similarity > 0.80)
- `cogz doctor` detects missing files and stale entities
- `cogz doctor --prune-observations` dry-run reports candidates
  correctly (rejected/superseded, aged, not active/stale)
- `cogz doctor --prune-observations --confirm` prunes observations,
  preserves tombstones, graph edges to tombstones remain valid
- Install script works on a clean system
- Binary size is reasonable (< 25 MB)
- Memory usage under 100 MB at idle
- Full end-to-end: install → init → index → MCP → search → context →
  hooks → update → uninstall

---

## MVP Definition

The **minimum viable product** is Phases 1-7. After Phase 7, CogZ
can:

- Be installed and initialized in a repo
- Store knowledge, rules, and observations as files
- Sync files to a per-repo SQLite DB
- Embed content with ONNX models
- Search with hybrid FTS + vector + RRF
- Assemble context packs with provenance
- Serve all MCP tools to an agent

What's missing from MVP: code indexing (Phase 8), consolidation
(Phase 9), stale flagging (Phase 10), hooks (Phase 11). These add
code-awareness and cognition but the core memory + context pipeline
works without them.

**Build order for MVP:** 1 → 2 → 3 → 4 → 5 → 6 → 7

**Full build order:** 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12

---

## Effort Estimates

Not time estimates — relative complexity per phase:

| Phase | Complexity | Why |
|---|---|---|
| 1. Skeleton + Config | Low | Standard Rust project setup |
| 2. Storage | Medium | Schema, CRUD, FTS5, vec0 integration |
| 3. File Layer | Medium | Frontmatter parsing, sync logic, hash-based change detection |
| 4. Embedding | Medium | ONNX Runtime integration, model download, caching |
| 5. Search | Medium | RRF fusion, graph expansion, provenance tracking |
| 6. Context | Medium | Mode logic, token budgeting, section prioritization |
| 7. MCP | Low | Protocol implementation, tool wiring (all layers exist) |
| 8. Code Indexing | High | Tree-sitter integration, multi-language, gitignore filtering |
| 9. Consolidation | High | NLI integration, promotion logic, merge with edge redirection |
| 10. Git Diff | Medium | Diff parsing, stale flagging, incremental reindex |
| 11. Hooks | Low | CLI commands that call existing context assembly |
| 12. Polish | Medium | Model download (hf-hub), cleanup, self-update, doctor, install script, CI, profiling |

The highest-complexity phases (8, 9) come after MVP. If we need to
ship something usable before the full build, MVP (phases 1-7) is a
coherent product.
