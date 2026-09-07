# Architecture

CogZ is a local-first, code-aware engineering cognition runtime. This document describes the system architecture, module responsibilities, and data flow.

## Design principles

1. **Local-first, always.** No cloud services, no remote APIs for core functionality, no telemetry. The only network access is optional model downloads. Every feature must work offline after initial setup.

2. **Files are canonical, DB is derived.** Every entity is a Markdown file. The SQLite database is a derived index — disposable and fully rebuildable. `cogz reset` + `cogz index` reconstructs everything from files and source code.

3. **Memory is the primary constraint.** Every component must justify its memory footprint. Model inference is isolated so it can be loaded, unloaded, and swapped without affecting the rest of the system.

4. **Graceful degradation.** The system must function without ONNX models. FTS-only mode is always available — hooks, context packs, consolidation (title-based dedup), doctor, and prune all work without models.

5. **Agent- and model-agnostic.** CogZ does not tie to any specific agent or model. The MCP interface is stateless per the 2026-07-28 MCP spec (SEP-2577) — no Roots, no sessions. Every tool call specifies which repo it targets via an explicit `repo` parameter. Models are configurable and swappable.

## Tech stack

| Component | Choice | Rationale |
|---|---|---|
| Language | Rust (edition 2024) | ~28 MB binary, ~11 MB idle RAM. Real threads. Single binary deployment. |
| Storage | SQLite (WAL mode) | Local-first, embedded, no server. Per-repo, not global. |
| Vector search | sqlite-vec | Same SQLite database, no separate vector store. |
| Full-text search | SQLite FTS5 | Same database. Porter + unicode61 tokenizer. |
| Code parsing | tree-sitter | Multi-language AST extraction. |
| Embedding inference | ort (ONNX Runtime) | Rust-native inference, dynamic loading. |
| MCP server | rmcp | Native MCP protocol implementation. |
| CLI | clap | Standard Rust CLI framework. |
| Config | serde + TOML | Typed, validated at startup. |

## Module map

```
src/
  main.rs              CLI entry point (clap)
  cli.rs               CLI command dispatch
  cli_embed.rs         CLI embedding helpers
  commands/            CLI command implementations
  config/              Typed config (TOML), validation
  storage/             SQLite layer
    schema.rs          Migrations, schema version
    crud.rs            Entity CRUD, EntityType enum
    edges.rs           Edge CRUD (graph relationships)
    embeddings.rs      vec0 embedding storage and KNN search
    events.rs          Domain event recording
    graph.rs           Graph traversal (BFS expansion)
    query.rs           Entity queries (by type, by reference)
    status.rs          Status state machine
    access.rs          Entity access tracking
  files/               File I/O and file→DB sync
    frontmatter.rs     Minimal YAML frontmatter parser
    entities.rs        EntityFile struct, read/write
    sync.rs            File→DB synchronization
    sync_ops.rs        Sync operations (create/update/stale)
    embed_sync.rs      Embedding computation and storage
  embed/               ONNX model loading and inference
    onnx.rs            Embedding model (CodeRankEmbed, bge-base)
    nli.rs             NLI model (contradiction detection)
    runtime.rs         ONNX Runtime initialization
    download.rs        Model download via hf-hub
    registry.rs        Model ID → HF source mapping
    cache.rs           Content-hash embedding cache
    model.rs           Traits: EmbeddingModel, NliModel
    suppress.rs        Stderr suppression during ONNX init
    similarity.rs      Cosine similarity, L2→cosine conversion
  search/              Hybrid search
    hybrid.rs          FTS + vector fusion
    rrf.rs             Reciprocal Rank Fusion
    expand.rs          Graph expansion (BFS)
    balance.rs         Source-type balancing (experimental)
    scoring.rs         Relevance scoring, recency decay
    describe.rs        Graph path description
  context/             Context pack assembly
    assemble.rs        Mode-specific assembly orchestrator
    modes.rs           ContextMode enum (cold_start, task, escalation)
    compress.rs        Token estimation, priority sorting, budget fitting
    code_map.rs        Cold-start code map generation
  consolidate/         Memory consolidation
    dedup.rs           Duplicate detection (title + embedding)
    contradict.rs      NLI-based contradiction detection
    promote.rs         Observation → rule promotion
    merge.rs           Duplicate merge with edge redirection
  index/               Code indexing
    tree_sitter.rs     AST extraction (7 languages)
    gitignore.rs       Gitignore-aware source file scanner
    git_diff.rs        Git diff-based change detection
    code_graph.rs      Structural edge sync (calls, imports, extends, contains)
    sync.rs            Code entity DB sync
    auto_link.rs       Knowledge → code auto-linking
    stale_flagging.rs  Stale knowledge flagging on code changes
  mcp/                 MCP server
    server.rs          ServerHandler impl, repo/model caching
    tools.rs           Tool router (13 tools)
    params.rs          Tool parameter structs
    helpers.rs         Shared helpers (file writes, response builders)
    entity_helpers.rs  Entity creation with dedup/contradiction
  hooks/               Lifecycle event handlers
    lifecycle.rs       Event dispatch, context pack assembly
    capture.rs         CLI capture-event handler
    handlers.rs        Event-specific handlers (file_save, session_end)
  doctor/              Health checks
    checks.rs          Doctor report, all check implementations
    checks_analysis.rs Extracted analysis helpers
  init.rs              cogz init
  update.rs            Self-update from GitHub releases
```

## Data flow

### Write path (MCP tool → file → DB)

```
Agent calls record_observation MCP tool
  → entity_helpers.rs: create_entity_file()
    → scan for secrets (reject if found)
    → write_entity_file_atomic() — writes Markdown file to .cogz/
    → sync_single_file() — syncs file to DB
    → embed_entity_text() — computes embedding (if model available)
    → store_embeddings() — stores embedding in vec0 table
    → check_duplicate() — title + embedding dedup
    → check_contradiction() — NLI contradiction detection
    → record_event() — domain event
  → return JSON response
```

The file is written first. If the file write fails, the DB is not updated. This is the file-first invariant.

### Read path (search → context pack)

```
Agent calls get_context MCP tool
  → assemble_context()
    → search::search() — hybrid FTS + vector with RRF fusion
      → FTS5 query (always available)
      → KNN vector query (if embeddings available)
      → RRF fusion of FTS + vector results
      → Graph expansion (BFS from matched entities)
    → sort_by_priority() — rules > observations > knowledge > code
    → fit_budget() — truncate/drop sections to fit token budget
  → return ContextPack JSON
```

### Index path (cogz index)

```
cogz index
  → files::sync_all() — sync .cogz/ files to DB
    → scan_entity_files() — walk .cogz/knowledge/, rules/, observations/
    → for each file: read, parse frontmatter, compute hash, sync to DB
    → mark stale: DB entities whose file was deleted
  → index::index_code() — index source code
    → scan_source_files() — walk repo, respect .gitignore + allow/deny
    → for each file: parse with tree-sitter, extract entities + edges
    → sync_code_entities() — insert/update/stale-mark in DB
    → sync_code_edges() — structural edges (calls, imports, extends, contains)
    → sync_auto_links() — knowledge → code auto-references
  → embed_synced() — embed all new/updated entities
  → record last_indexed_commit for incremental reindex
```

### Hook path (lifecycle event)

```
Agent fires session_start hook
  → cogz capture-event session_start --hook-json --fts-only
    → parse event type
    → handle_lifecycle_event()
      → record_event() — audit trail
      → assemble_pack() — cold_start context pack
        → embed query (skipped with --fts-only)
        → assemble_context() — FTS-only retrieval
      → return context pack
    → print JSON to stdout: {"hookSpecificOutput": {...}}
```

## Concurrency model

- **Single SQLite `Connection` behind `std::sync::Mutex`.** No connection pool, no `r2d2`, no `tokio-rusqlite`.
- **DB calls from async MCP handlers go through `tokio::task::spawn_blocking`.** Never call rusqlite directly from an async context.
- **Don't hold the mutex during filesystem or network I/O.** Acquire the lock, get the data, drop the lock, then do the I/O.
- **Models are lazy-loaded and shared.** The MCP server caches model instances across repos. Models auto-unload after `model_idle_ttl` seconds of inactivity.

## Database

The database lives at `.cogz/cogz.db` (configurable via `storage.db_path`). It is gitignored and fully rebuildable.

**Tables:**
- `entities` — all entities (file-backed and code)
- `edges` — graph relationships
- `entities_fts` — FTS5 virtual table (synced via triggers)
- `code_embeddings` — vec0 virtual table for code entity embeddings
- `knowledge_embeddings` — vec0 virtual table for knowledge entity embeddings
- `events` — domain event log
- `meta` — key-value metadata (e.g. `last_indexed_commit`)
- `entity_access` — access tracking (derived)

See [Schema](../dev/schema.md) for the full schema and migration history.

## Entity types

| Type | Source | File-backed | UUID | Description |
|---|---|---|---|---|
| observation | Agent | yes | v4 | Raw, unvalidated experience |
| rule | Agent | yes | v4 | Validated directive |
| knowledge | Agent | yes | v4 | Structured documentation |
| function | tree-sitter | no | v5 | Code entity |
| class | tree-sitter | no | v5 | Code entity |
| file | tree-sitter | no | v5 | Code entity |
| module | tree-sitter | no | v5 | Code entity |

See [Entity Model](entity-model.md) for frontmatter schema, state machine, and update policy.

## Edge types

| Edge type | Source | Description |
|---|---|---|
| `references` | Frontmatter | Entity references another entity or file path |
| `auto_references` | Auto-linking | Knowledge → code auto-link (DB-only, rebuildable) |
| `supports` | Agent | Observation supports another observation (for promotion) |
| `derived_from` | Promotion | Rule derived from an observation |
| `superseded_by` | Merge | Superseded entity points to survivor |
| `contradicts` | NLI | Entity contradicts another entity |
| `calls` | tree-sitter | Function calls another function |
| `imports` | tree-sitter | Module/file imports another |
| `extends` | tree-sitter | Class extends another class |
| `contains` | tree-sitter | File/module contains a function/class/module |

## See also

- [Entity Model](entity-model.md) — entity types, frontmatter, state machine
- [Search](search.md) — hybrid FTS + vector, RRF, graph expansion
- [Consolidation](consolidation.md) — dedup, contradiction, promotion, merge
- [Degradation](degradation.md) — FTS-only mode and fallback behavior
- [Schema](../dev/schema.md) — DB schema and migrations
- [Conventions](../dev/conventions.md) — code patterns and invariants
