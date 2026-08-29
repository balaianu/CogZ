# CogZ — Base Architecture

This document defines the concrete architecture derived from the goal
and first principles. It is the reference for all implementation
decisions.

---

## Tech Stack

| Component | Choice | Rationale |
|---|---|---|
| Language | Rust | 50-100MB runtime vs Python's 400-600MB. Real threads. Single binary deployment. 10-100x faster on CPU-bound work (graph traversal, RRF, context assembly). Critical on 2012-era FX-8320. |
| Storage | SQLite (WAL mode) | Local-first, embedded, no server. Mature, reliable, portable. Per-repo, not global. |
| Vector search | sqlite-vec | Same SQLite database, no separate vector store. |
| Full-text search | SQLite FTS5 | Same database. One FTS table with type filter. |
| Code parsing | tree-sitter (Rust bindings) | Multi-language AST extraction. Mature Rust crate. |
| Embedding inference | ort (ONNX Runtime) | Same models as v1 (CodeRankEmbed, bge-base), Rust-native inference. |
| MCP server | mcp Rust SDK | Native MCP protocol implementation. |
| CLI | clap | Standard Rust CLI framework. |
| Config | serde + TOML | Typed, validated at startup, single file. |
| Serialization | serde + JSON | For entity properties and event payloads. |

---

## Database: Per-Repo

The database lives inside the repo at `.cogz/cogz.db`. It is
gitignored (binary, machine-generated, fully rebuildable).

**Why per-repo, not global:**

- The repo boundary is the scope boundary. No taxonomy filter needed.
- Knowledge files are per-repo (`.cogz/knowledge/`). A global DB
  indexing per-repo files creates a mismatch — orphaned entries
  accumulate when repos are deleted.
- A per-repo DB is smaller (only that repo's entities). On a 7 GB
  machine, keeping the working set small matters.
- Fresh clone + `cogz index` = full memory rebuild from files + code.
  A global DB can't do this cleanly.
- Cross-repo search was never used in CogZ-py. It added complexity for
  a capability nobody used. If needed later, a `cogz search --global`
  mode could query multiple DBs.

**No subcategory.** CogZ-py's subcategory was always the repo name
wearing a disguise. With a per-repo DB, the scoping is implicit. The
`project` name is kept in config as a stable identifier (autodetected
on init, saved, versioned) to ensure things don't break on project
folder renames — but it is metadata, not a query filter.

---

## Module Structure

```
cogz/
  src/
    main.rs              — CLI entry point (clap)
    commands.rs          — CLI command handlers (status, index, reindex, reset)
    cli.rs               — CLI helpers (embedding, search, context, MCP)
    lib.rs               — library root, public API

    config/
      mod.rs             — config loading, validation
      settings.rs        — typed config structs (serde)

    storage/
      mod.rs             — storage root
      schema.rs          — table definitions, migrations
      crud.rs            — entity CRUD operations
      query.rs           — read queries (by type, graph, status)
      events.rs          — domain event recording
      status.rs          — status state machine validation

    index/
      mod.rs             — orchestration: scan → parse → sync → edges
      gitignore.rs       — gitignore-aware source file scanner
      tree_sitter.rs     — shared types, Rust entity extraction
      tree_sitter/
        python.rs        — Python entity extraction
      sync/
        mod.rs           — code entity sync (UUID v5, content hash, stale)
      code_graph/
        mod.rs           — Rust edge extraction + shared utilities
        python.rs        — Python edge extraction

    embed/
      mod.rs             — embedding root
      model.rs           — EmbeddingModel trait
      onnx.rs            — ONNX implementation
      cache.rs           — embedding cache (avoid re-embedding unchanged content)

    search/
      mod.rs             — search root
      hybrid.rs          — FTS5 + vec search
      rrf.rs             — reciprocal rank fusion
      expand.rs          — graph-aware expansion from search results

    context/
      mod.rs             — context root
      assemble.rs        — context pack assembly
      compress.rs        — token budget management, section prioritization
      modes.rs           — cold_start, task, escalation mode definitions

    consolidate/
      mod.rs             — consolidation root
      dedup.rs           — duplicate detection (embedding similarity + title match)
      merge.rs           — entity merging, edge redirection
      promote.rs         — observation → rule promotion
      contradict.rs      — NLI-based contradiction detection

    files/
      mod.rs             — file layer root
      entities.rs        — markdown file I/O, frontmatter parsing for all entity types
      sync.rs            — file → database synchronization (one-directional)

    mcp/
      mod.rs             — MCP root
      server.rs          — MCP protocol server
      tools.rs           — tool definitions (13 tools)

    hooks/
      mod.rs             — hooks root
      lifecycle.rs       — session_start, prompt_submit, pre/post_tool_use
      capture.rs         — event capture from external scripts
```

### File size rule

No source file exceeds 400 lines. If a file grows past this, split it
by responsibility. This is a structural constraint, not a guideline.
CogZ-py's `storage.py` reached 2787 lines and caused cascading
complexity (parameter count 6.6x over threshold, nesting depth
requiring 7 iterations to fix). This does not happen again.

---

## Entity Model: Everything Is a File

### The invariant

**All entities are markdown files on disk. The database is a derived
index. There is no DB-canonical content. `cogz reindex` rebuilds the
entire DB from files + code.**

This is the most important architectural decision. It eliminates:

- Sync conflicts (files are always canonical)
- Data loss on DB corruption (files survive, DB is disposable)
- Ambiguity about what's the source of truth (always the file)

### Write path

```
MCP/CLI call → write file to disk → sync DB from file
              (if file write fails, DB is not updated — no inconsistency)
```

There is no code path that writes to the DB without first writing a
file. The DB is never a write target. It's a read index.

### Entity types and their file locations

| Entity type | File location | Git-tracked? | Who creates |
|---|---|---|---|
| knowledge | `.cogz/knowledge/<category>/` | Yes | Human (direct edit) or agent (`create_knowledge`) |
| rule | `.cogz/rules/` | Yes | Agent (`create_rule`) or consolidation (promotion) |
| observation | `.cogz/observations/<year-month>/` | No (gitignored) | Agent (`record_observation`) |
| function | (code file, not a .cogz file) | N/A — indexed from source | Indexer (tree-sitter) |
| class | (code file, not a .cogz file) | N/A — indexed from source | Indexer (tree-sitter) |
| file | (code file, not a .cogz file) | N/A — indexed from source | Indexer (tree-sitter) |
| module | (code file, not a .cogz file) | N/A — indexed from source | Indexer (tree-sitter) |

Code entities (function, class, file, module) are not stored as
markdown files — they're extracted from source code by tree-sitter
and exist only in the DB. They are rebuildable from source via
`cogz index`. Source code is the canonical store for code entities.

### What is version-tracked

| Path | Git-tracked? | Why |
|---|---|---|
| `.cogz/config.toml` | Yes | Shared config — project identity, search settings, consolidation thresholds. Team members need the same config. |
| `.cogz/knowledge/` | Yes | Human-readable, curated documentation. Shared understanding of the codebase. Meant to be read, reviewed, versioned. |
| `.cogz/rules/` | Yes | Validated knowledge. Trustworthy, stable, shared. A team should agree on rules. |
| `.cogz/observations/` | No (gitignored) | Raw experience from a specific agent. Personal, transient, potentially wrong. Not shared. Survives on disk but not versioned. |
| `.cogz/cogz.db` | No (gitignored) | Binary, machine-generated, fully rebuildable from files + code. |

**Principle: git tracks human-readable canonical content. Everything
derived or transient is gitignored.**

### File → DB sync (one-directional)

Sync is file-based, not git-based. Git status is irrelevant to the
sync. The sync scans `.cogz/` on disk — if a file exists, it gets
indexed. Whether git tracks it or not doesn't matter.

```
.cogz/ directory     → always sync (file-based, git-agnostic)
repo source files    → respect .gitignore (git-aware)
```

**Sync rules:**

- On `cogz index` / `cogz reindex`: scan all files in `.cogz/knowledge/`,
  `.cogz/rules/`, `.cogz/observations/`. For each file:
  - New file → create DB entity, embed, index in FTS
  - Changed file (content hash differs) → update DB entity, re-embed, re-index
  - Deleted file → mark DB entity `status = 'stale'` (don't delete — preserve graph edges for audit)
- On MCP/CLI write (create entity): write file first, then sync DB from file
- On MCP/CLI edit (update entity): modify file first, then sync DB from file

**No bidirectional sync.** Files are canonical. DB is derived. There
is no "DB writes back to file" path except on initial creation.

### Code indexing: gitignore-aware

Tree-sitter indexing respects `.gitignore`. Gitignored files are not
indexed as code entities. This prevents:

- Indexing secrets (`.env`, API keys)
- Indexing noise (`node_modules/`, `.venv/`, dependencies)
- Indexing churn (generated files that change on every build)

An explicit allowlist can override gitignore for specific paths:

```toml
[index]
allow = ["proto/generated/*.rs"]  # index despite being gitignored
```

Default: respect `.gitignore`. No special configuration needed for
the common case.

### Entity file format

All entity files are markdown with YAML frontmatter:

```markdown
---
id: <uuid>
title: "Search Architecture"
type: knowledge
status: active
created_at: 2026-08-27T09:00:00Z
updated_at: 2026-08-27T09:00:00Z
references: ["a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023"]  # UUIDs of referenced entities
tags: [search, fts5, vector]
---

# Search Architecture

The search system uses hybrid FTS5 + vector search with RRF fusion...
```

Observations have the same format but are smaller and in
`.cogz/observations/`:

```markdown
---
id: <uuid>
title: "FTS5 ranking bug in _build_fts_search_sql"
type: observation
status: active
created_at: 2026-08-27T14:30:00Z
updated_at: 2026-08-27T14:30:00Z
references: ["a1b2c3d4-e5f6-4789-abcd-000000000042"]  # UUID of the referenced function
source: agent
---

The RRF fusion in _build_fts_search_sql produces incorrect rankings
when k=60 and the FTS result set is smaller than the vector result set.
```

### Fresh clone behavior

A new contributor clones the repo. They get:
- `.cogz/config.toml` — project config
- `.cogz/knowledge/` — curated documentation
- `.cogz/rules/` — validated rules

They do NOT get:
- `.cogz/observations/` — gitignored, personal to the original agent
- `.cogz/cogz.db` — gitignored, will be rebuilt

They run `cogz index`:
1. Tree-sitter parses code → code entities + structural edges
2. Scans `.cogz/knowledge/` and `.cogz/rules/` → knowledge/rule entities
3. Generates embeddings for all entities
4. Builds FTS index
5. DB is fully populated from files + code

Clean start with curated knowledge. No raw observations from someone
else's agent sessions.

---

## Storage Schema

### Design: one table per concern

CogZ-py had 11 typed entity tables + a unified nodes table = dual
storage, 12 FTS5 tables, double write path. CogZ has one entity table,
one edge table, one FTS table, one embedding table, one events table.

### Schema

Entity IDs are UUID v4 strings throughout the system — in file
frontmatter, in the database, in MCP tool parameters and returns, and
in `references` fields. This ensures files are self-contained and
portable: a `references` list in a frontmatter file points to the same
entity after a DB rebuild. Code entities (functions, classes, files,
modules) get deterministic UUID v5 values derived from
`{file_path}:{entity_type}:{qualified_name}`; they are
stored in the DB only (no file on disk).

```sql
-- All entities: observations, rules, knowledge, functions, classes, files, modules
CREATE TABLE entities (
    id          TEXT PRIMARY KEY,         -- UUID v4 (file-backed) or v5 (code entities)
    type        TEXT NOT NULL,           -- 'observation', 'rule', 'knowledge',
                                         -- 'function', 'class', 'file', 'module'
    title       TEXT,
    content     TEXT NOT NULL,
    properties  TEXT DEFAULT '{}',       -- JSON: type-specific fields
                                         -- (confidence, supporting_ids, file_path, etc.)
    file_path   TEXT,                    -- path to canonical file (NULL for code entities
                                         -- extracted from source, which use properties.file_path)
    status      TEXT DEFAULT 'active',   -- active, stale, superseded, rejected, pruned
    content_hash TEXT,                   -- hash of file content for change detection
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX idx_entities_type ON entities(type);
CREATE INDEX idx_entities_status ON entities(status);
CREATE INDEX idx_entities_file_path ON entities(file_path);

-- Full-text search: one table, type-filtered at query time
-- FTS5 external content uses implicit rowid; UUID is looked up via entities.id
CREATE VIRTUAL TABLE entities_fts USING fts5(
    title,
    content,
    content='entities',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

-- Graph edges: all relationships in one table
CREATE TABLE edges (
    source_id   TEXT NOT NULL REFERENCES entities(id),
    target_id   TEXT NOT NULL REFERENCES entities(id),
    edge_type   TEXT NOT NULL,           -- 'calls', 'imports', 'extends',
                                         -- 'references', 'supports',
                                         -- 'contradicts', 'derived_from'
    weight      REAL DEFAULT 1.0,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (source_id, target_id, edge_type)
);

CREATE INDEX idx_edges_source ON edges(source_id, edge_type);
CREATE INDEX idx_edges_target ON edges(target_id, edge_type);

-- Embeddings: one vec0 table for all entity types
-- entity_id is a TEXT column (UUID) mapping to entities.id
CREATE VIRTUAL TABLE entity_embeddings USING vec0(
    embedding FLOAT[768],
    entity_id TEXT
);

-- Domain events only (never indexing operations)
CREATE TABLE events (
    id          INTEGER PRIMARY KEY,      -- auto-increment event sequence
    event_type  TEXT NOT NULL,           -- 'observation_created',
                                         -- 'observation_edited',
                                         -- 'observation_rejected',
                                         -- 'rule_created',
                                         -- 'rule_edited',
                                         -- 'rule_promoted',
                                         -- 'knowledge_created',
                                         -- 'knowledge_updated',
                                         -- 'knowledge_merged',
                                         -- 'contradiction_found',
                                         -- 'code_changed'
    entity_id   TEXT REFERENCES entities(id),
    payload     TEXT DEFAULT '{}',       -- JSON
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_events_type ON events(event_type);
CREATE INDEX idx_events_entity ON events(entity_id);
```

### What this eliminates vs CogZ-py

| CogZ-py | CogZ |
|---|---|
| 11 typed tables + 1 nodes table | 1 entity table |
| 12 FTS5 virtual tables | 1 FTS5 table |
| Events table 35% of DB (indexing noise) | Domain events only |
| Double write path (typed + nodes) | Single insert (from file) |
| Double embedding surface | Single embedding table |
| Consolidation updates both representations | Consolidation updates one |
| Global DB with taxonomy scoping | Per-repo DB, no taxonomy filter |
| project + subcategory + domain columns | None — repo boundary is the scope |

### Type-specific fields in properties JSON

Fields that were columns in CogZ-py's typed tables move to the
`properties` JSON column:

| Entity type | Properties fields |
|---|---|
| observation | `confidence`, `supporting_ids`, `source` |
| rule | `confidence`, `supporting_ids`, `validation_count`, `superseded_by` |
| knowledge | `category`, `tags` |
| function | `file_path`, `line_start`, `line_end`, `language`, `signature` |
| class | `file_path`, `line_start`, `line_end`, `language` |
| file | `path`, `language`, `line_count` |
| module | `path`, `language` |

SQLite's JSON1 extension provides `json_extract()` for querying these
when needed. The vast majority of queries filter by `type` and
`status` — both top-level columns.

### Concurrency model

The MCP server is async (tokio + rmcp). DB operations are synchronous
(rusqlite). The bridge is simple:

- **Single `Connection` behind `std::sync::Mutex`.** CogZ is not a
  high-concurrency web server — it's a single-agent memory tool making
  sequential tool calls. One connection is sufficient.
- **`tokio::task::spawn_blocking` for DB calls from async handlers.**
  This prevents blocking the tokio runtime on DB operations. Most
  queries are sub-millisecond; `spawn_blocking` ensures even complex
  searches (graph traversal, FTS + vec fusion) don't stall the
  async executor.
- **WAL mode** allows concurrent readers + one writer at the SQLite
  level. With a single connection, this doesn't add concurrency, but
  WAL is still preferred for crash recovery and checkpoint behavior.
- **No connection pool.** No `r2d2`, no `deadpool`, no `tokio-rusqlite`.
  These add complexity and dependencies for a workload that doesn't
  need them. If concurrency becomes a bottleneck (multiple agents on
  the same repo), upgrading to a read/write split is a contained
  change in `storage/`.

```rust
// The storage layer owns this. MCP handlers borrow it via spawn_blocking.
pub struct Storage {
    conn: Mutex<Connection>,
}

// MCP handler (async):
let entities = tokio::task::spawn_blocking(move || {
    storage.search_entities(query)
}).await??;
```

This is the Rust-idiomatic way to use sync code from async. No new
dependencies. The Mutex serializes access (fine for our workload),
and `spawn_blocking` keeps the async runtime responsive.

---

## Model Layer

### Trait-based isolation

```rust
pub trait EmbeddingModel: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn dimension(&self) -> usize;
    fn model_name(&self) -> &str;
}

pub trait RerankerModel: Send + Sync {
    fn rerank(&self, query: &str, docs: &[String]) -> Result<Vec<(usize, f32)>>;
}

pub trait NliModel: Send + Sync {
    fn classify(&self, premise: &str, hypothesis: &str) -> Result<NliLabel>;
}
```

The storage, search, consolidation, and context modules depend on the
traits, never on concrete implementations. The runtime wires
implementations based on config at startup.

### Implementations

- `onnx.rs` — ONNX Runtime inference for local models
  (CodeRankEmbed, bge-base, NLI models)
- Future: `api.rs` — HTTP-based embedding API (OpenAI, etc.) if needed

### Process isolation

The embedding model loads in a dedicated thread. If the model crashes
or is unloaded, the MCP server continues serving from cache and
FTS-only search. This is possible because Rust has real threads, not
a GIL.

### Graceful degradation

| Model available | Capabilities |
|---|---|
| Embedding + NLI + Reranker | Full: hybrid search, dedup (title + embedding), contradiction, reranking |
| Embedding only | Hybrid search, dedup (title + embedding). No contradiction detection, no reranking. |
| None | FTS-only search. Title-based dedup only (exact + fuzzy). No embedding dedup, no contradiction, no vector search. |

The system always functions. Missing models reduce capability, not
availability.

---

## Code-Awareness

### Indexing pipeline

```
Repository → tree-sitter parse → extract entities → build structural edges
     (respecting .gitignore)              ↓
                            functions, classes, files, modules
                            inserted as entities (type field)
                                         ↓
                            edges: calls, imports, extends
                            between code entities
```

Code entities live in the same `entities` table as observations and
rules. Same ID space, same graph, same FTS table, same embedding
table. There is no "code graph" and "knowledge graph" — there is one
graph.

### Knowledge-to-code edges

When an observation or rule is created, it can reference code entities:

```
observation: "The FTS5 ranking in _build_fts_search_sql has a bug
              with RRF fusion when k=60"
    ↓ references edge
function entity: _build_fts_search_sql
    ↓ calls edge
function entity: _fts_search_all
```

Retrieval for "FTS5 ranking" finds the observation, follows the
`references` edge to the function, follows `calls` edges to related
functions, and returns the full picture: the observation, the code
it's about, and the surrounding call graph.

### Git diff → stale knowledge flagging

When code changes (detected via git diff), the indexer:

1. Identifies which code entities were modified or deleted
2. Finds all observations/rules/knowledge with `references` edges to
   those entities
3. Marks them `status = 'stale'`
4. Emits a `code_changed` domain event

The agent is then notified that its knowledge about certain code
regions may need review. This is the cognition layer: the system
knows when its memory might be wrong.

---

## Search

### Hybrid FTS5 + vector search with RRF fusion

```
Query
  ├── FTS5 search (entities_fts, filtered by type)
  ├── Vector search (entity_embeddings, filtered by type)
  └── RRF fusion → ranked results
        ↓
  Graph expansion (follow edges from top results)
        ↓
  Expanded result set with graph paths
```

No taxonomy WHERE filter — the DB is per-repo, so all entities belong
to the current repo. Queries filter by `type` (code vs knowledge) to
prevent knowledge entries from drowning out code results, and by
`status` (default: `active` only — stale/superseded/rejected entities
are excluded from search and context unless explicitly requested).

### Multi-model embedding

Code entities and knowledge entities use different embedding models
(inherited from CogZ-py):

- Code: `nomic-ai/CodeRankEmbed-int8` (768d, code-specific)
- Knowledge: `BAAI/bge-base-en-v1.5` (768d, general-purpose)

Both share the same vec0 table (same dimension). The `type` field
distinguishes which model produced the embedding. Search runs both
code and knowledge queries separately and merges, preventing knowledge
entries from drowning out code results.

> **Phase 7 note:** Model selection is hardcoded via `ModelType` in
> `src/embed/onnx.rs`. The `code_model` and `knowledge_model` config
> fields are metadata for status display only — they are not yet wired
> to model loading. Configurable model selection arrives with Phase 8
> (multi-model embedding for code indexing).

---

## Context Assembly

### The primary output

```rust
pub struct ContextPack {
    pub query: String,
    pub mode: ContextMode,
    pub sections: Vec<ContextSection>,
    pub metadata: PackMetadata,
}

pub struct ContextSection {
    pub source: String,            // "observation", "rule", "function", etc.
    pub entity_id: String,         // UUID
    pub title: String,             // entity title (for display)
    pub content: String,
    pub relevance: f32,
    pub graph_path: Vec<String>,   // UUIDs showing how we got here
}

pub struct PackMetadata {
    pub size_tokens: usize,
    pub selected_sources: Vec<String>,   // source types of included sections
    pub dropped_sources: Vec<String>,    // "source:title (reason)" for dropped sections
    pub search_mode: String,             // "hybrid" or "fts_only"
}
```

The `graph_path` field is the key innovation. It tells the agent *why*
each piece of context was included:

> "This function was included because your query matched an
> observation (UUID 550e8400...) that references it (UUID a1b2c3d4...),
> and this function calls another function (UUID a1b2c3d4...) that was
> also relevant."

This makes the cognition layer's reasoning visible and auditable.

### Token counting

Context packs have a token budget (configurable, default from config).
The token count is an **approximation**, not an exact count:

- **chars/4 heuristic**: `tokens ≈ len(text) / 4` for UTF-8 bytes.
  Within ~15% for English prose, overestimates for code (safe — packs
  come in under budget).
- **No external tokenizer dependency.** CogZ is model-agnostic — the
  target agent's tokenizer is the real source of truth. We just need
  "will this approximately fit?" The chars/4 heuristic is a single
  function in `context/compress.rs`, zero dependencies.
- **Budget gates fail safe.** If the estimate is wrong, the pack is
  smaller than it could be (wasteful but safe), never larger (which
  would overflow the agent's context window).
- **Upgrade path.** If the approximation proves too wasteful in
  practice, swap the `estimate_tokens()` function for `tokenx-rs`
  (96% accuracy, zero deps) or `tiktoken-rs` (exact, OpenAI-specific).
  The function is behind a trait; the rest of the system is unchanged.

### Modes

| Mode | When used | Behavior |
|---|---|---|
| `cold_start` | Session start | Compact pack: repo identity, active constraints, recent validated rules, recent observations. No broad code inventory. |
| `task` | Per prompt | Ranked retrieval using the agent's query. Graph expansion from matched entities. |
| `escalation` | When task pack insufficient | Wider retrieval: larger k, more expansion hops, lower relevance threshold. |

---

## Consolidation

### Continuous, not batch

Every insert triggers lightweight consolidation:

| Phase | When | What |
|---|---|---|
| Dedup | On insert | Compare new entity to existing entities of same type via embedding similarity + title match. Flag if similarity > threshold (0.92). Return `duplicate_warning` to caller. |
| Contradict | On insert | NLI model checks new entity against related existing entities. Flag contradictions. |
| Promote | Background | Observation with sufficient support (multiple supporting observations, code structure validation) → promoted to rule. |
| Merge | Background | Confirmed duplicates merged: one entity superseded, edges redirected to survivor. |

No 7-phase batch pipeline. No "consolidation hasn't run in weeks."
Dedup and contradiction detection happen immediately. Promotion and
merge happen in the background when thresholds are met.

### Duplicate detection stack

| Check | When | Cost | What it catches |
|---|---|---|---|
| Exact title match | On create | Negligible | Same title, almost certainly duplicate |
| Fuzzy title match | On create | Low | Similar titles, likely related |
| Embedding similarity > 0.92 | On create (dedup hook) | One embedding + KNN query | Semantic duplicates |
| Embedding similarity > 0.80 | On `cogz doctor` | Pairwise, expensive | Near-duplicates that slipped through |

Knowledge duplicates are flagged, not auto-resolved — knowledge is
human-curated and the system shouldn't silently merge two architecture
docs. Observation duplicates can be auto-merged by consolidation. Rule
duplicates are surfaced for review.

---

## Retention and Bounded Growth

Observations are append-only, gitignored, and numerous. After months
of daily use, thousands accumulate — most stale or rejected. The
system must not grow unboundedly (first principle #10), but memory is
additive (first principle #8). The resolution: **explicit pruning via
`cogz doctor`, with tombstones for graph integrity.**

### What can be pruned

| Status | Prunable? | Why |
|---|---|---|
| `active` | Never | Current, valid memory |
| `stale` | No | May still be useful context; code might revert |
| `rejected` | Yes (after age threshold) | Contradicted, terminal — no longer useful |
| `superseded` | Yes (after age threshold) | Replaced, terminal — history preserved in graph edges |

Only `rejected` and `superseded` observations are prunable. Rules and
knowledge are never pruned — they are git-tracked, human-curated, and
few in number.

### Pruning mechanism

`cogz doctor --prune-observations` is the sanctioned command:

1. **Default: dry-run.** Reports counts by status and age, lists
   candidates for pruning. No changes made.
2. **`--confirm`:** Deletes observation files, removes DB entity
   content/embeddings/FTS entries, but preserves a **tombstone** — a
   minimal DB record with `id`, `type`, `status='pruned'`,
   `deleted_at`. Edges to/from the tombstoned entity remain valid.
3. **Age threshold:** Only observations older than
   `observation_prune_after_days` (default: 90) are candidates.
   Configurable in `config.toml`.

### Tombstones

Tombstones keep the graph intact. An observation that was rejected
may still be referenced by a `contradicts` edge from the observation
that replaced it. Pruning the rejected observation without a tombstone
would orphan that edge. The tombstone preserves the relationship:

```sql
-- Tombstone: minimal record, no content, no embedding
-- Edges referencing this UUID remain valid
id='550e8400-e29b-41d4-a716-446655440042', type='observation', status='pruned', content='', deleted_at='2026-11-27T...'
```

Tombstones are bounded: `tombstone_max_count` (default: 1000). When
exceeded, oldest tombstones are removed. At that point, edges
referencing them are also removed — the history is considered old
enough to be irrelevant.

### Config

```toml
[retention]
observation_prune_after_days = 90  # prune rejected/superseded observations older than this
tombstone_max_count = 1000         # cap on tombstones; oldest removed when exceeded
```

### What this is not

- **Not automatic.** Pruning never happens without an explicit
  `cogz doctor --prune-observations --confirm`. The system grows if
  you don't prune — that's acceptable.
- **Not TTL-based.** Age is a threshold for candidacy, not a trigger.
  The human decides when to prune.
- **Not for rules or knowledge.** Those are git-tracked and few.
  Bounded growth is not a concern for them.

---

## Update Policy and Enforcement

### Per entity type

| Entity | Content update | Status update | Why |
|---|---|---|---|
| Knowledge | In-place (`update_knowledge`) | In-place | Documentation — current version matters, git preserves history |
| Rule (minor edit) | In-place (human file edit) | In-place | Wording fix, directive unchanged |
| Rule (substantive) | Supersede + new | In-place | Directive changed — DB needs both versions for cognitive continuity |
| Observation | Append-only (new observation) | In-place | Raw experience — corrections are new observations, not rewrites |

**Unifying principle:** memory is additive, with state transitions.
Destructive edits are only allowed when git preserves the history
(knowledge, rules) or when the edit is metadata, not content (status
changes).

### Status state machine

```
active → stale       (code changed)
active → rejected    (contradicted)
active → superseded  (replaced by new rule)
stale  → active      (code reverted)
rejected → pruned    (doctor --prune-observations)
superseded → pruned  (doctor --prune-observations)
rejected → (terminal otherwise)
superseded → (terminal otherwise)
pruned → (terminal — tombstone)
```

Rejected and superseded are terminal states. To revert, create a new
entity that supersedes the superseder. The history is preserved in the
graph. `pruned` is a terminal state for tombstoned entities — the
entity's content is gone but its ID and edges remain for graph
integrity. See Retention and Bounded Growth above.

### Enforcement layers

1. **MCP tool surface:** No `update_observation` or `edit_rule` tool
   exists. The agent can only create new observations and rules.
   `update_knowledge` is the only content-edit tool.
2. **File sync:** When reindex detects content changes, policy is
   applied per entity type. Observation content edits are logged as
   `observation_edited` events (audit trail). Rule content edits are
   logged as `rule_edited` events.
3. **Status state machine:** `transition_status()` validates all
   status transitions. Illegal transitions (e.g., `rejected → active`)
   are rejected with an error.
4. **Doctor:** `cogz doctor` detects policy violations that bypassed
   the tool surface — content edits on observations, illegal status
   transitions, orphaned supersedes, missing files. Reports without
   auto-fixing; the human decides.

---

## MCP Interface

### 13 tools

```
record_observation      query_observations
create_rule             query_rules
create_knowledge        update_knowledge
query_knowledge         search
get_context             consolidate
get_status              capture_event
list_entities
```

### Design rules

- Verb-first naming: `<verb>_<noun>` (inherited from CogZ-py, still correct)
- `get_` returns a singleton/summary
- `query_` returns a filtered collection
- `list_` enumerates all items of a kind
- `update_` edits an existing entity in-place (only knowledge — observations and rules are append-only)
- Flat namespace, no group prefixes

---

## CLI

```
cogz init                    — initialize .cogz/ in a repo (autodetect project name, generate config.toml)
cogz index                   — index the repository (code + .cogz/ files)
cogz search <query>          — search entities
cogz context [--mode] [q]    — assemble context pack
cogz consolidate             — trigger background consolidation
cogz status                  — show system status
cogz mcp-stdio               — run MCP server (stdin/stdout)
cogz capture-event <type>    — capture a lifecycle event (hook scripts)
cogz record-observation      — record an observation
cogz create-rule             — create a rule
cogz create-knowledge        — create a knowledge entry
cogz update-knowledge <id>   — update a knowledge entry (opens $EDITOR)
cogz reindex                 — re-index changed files
cogz doctor                  — health check (policy violations, stale entities, near-duplicates, missing files)
cogz doctor --prune-observations — dry-run: report prunable observations (rejected/superseded, aged)
cogz doctor --prune-observations --confirm — prune observations, preserve tombstones
cogz reset                   — drop DB only (safe, rebuildable)
cogz reset --purge           — drop DB + observations + generated .gitignore (keeps knowledge/rules/config)
```

Pinned commands (referenced by external scripts, must not rename):
- `cogz mcp-stdio` — referenced in MCP config files
- `cogz capture-event` — called by hook shell scripts

---

## Configuration

Generated by `cogz init` with defaults. Project name autodetected
from repo directory or git remote.

```toml
# .cogz/config.toml

[project]
name = "cogz"                    # autodetected on init, saved, versioned
                                 # stable identifier — survives folder renames

[storage]
db_path = ".cogz/cogz.db"        # per-repo database

[embedding]
code_model = "nomic-ai/CodeRankEmbed-int8"   # metadata only in Phase 7 — model selection is fixed
knowledge_model = "BAAI/bge-base-en-v1.5"    # metadata only in Phase 7 — model selection is fixed
dimension = 768

[search]
fts_weight = 0.4
vec_weight = 0.6
rrf_k = 60
max_results = 20

[consolidation]
dedup_threshold = 0.92
title_match_threshold = 0.85     # Levenshtein-based fuzzy title match
contradiction_check = true
promotion_threshold = 3          # min supporting observations to promote

[index]
allow = []                       # explicit gitignore overrides (paths to index despite being gitignored)

[context]
default_token_budget = 4096      # default token budget for context packs
cold_start_rules = 5             # recent rules in cold_start mode
cold_start_observations = 5      # recent observations in cold_start mode
task_max_results = 10            # max search results in task mode
task_max_hops = 2                # graph expansion hops in task mode
escalation_max_results = 20      # max search results in escalation mode
escalation_max_hops = 3          # graph expansion hops in escalation mode

[retention]
observation_prune_after_days = 90  # prune rejected/superseded observations older than this
tombstone_max_count = 1000         # cap on tombstones; oldest removed when exceeded
```

Typed via serde. Validated at startup. No env var sprawl. No Dynaconf
layering. One file, one source of truth.

---

## Deployment

Single static binary. No virtual environment, no package manager, no
path dependencies.

```
cogz                    — the binary (~10-20 MB)
.cogz/                  — per-repo config, knowledge, rules, observations, DB
  config.toml           — git-tracked
  knowledge/            — git-tracked
  rules/                — git-tracked
  observations/         — gitignored
  cogz.db               — gitignored
~/.local/share/cogz/    — model cache (ONNX models downloaded on first run)
```

The binary is placed in `~/.local/bin/` or anywhere on PATH. MCP
config references the binary path. Hook scripts call the binary
directly. No wrapper scripts needed.
