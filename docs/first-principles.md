# CogZ — First Principles

These are the axiomatic beliefs that guide every architectural and
implementation decision. They are not preferences or conventions —
they are the foundation. If a design choice contradicts a principle,
the design choice is wrong.

## 1. Local-first, always

The system runs entirely on the local machine. No cloud services, no
remote APIs for core functionality, no telemetry, no accounts. The
only network access is optional model downloads on first run.

This is non-negotiable. A cognition runtime that depends on external
infrastructure is not a cognition runtime — it's a client. CogZ must
function fully offline after initial setup.

**Implication:** Every feature must work within the constraints of the
host machine. If a feature cannot work locally, it does not ship.

## 2. Memory is the primary constraint

The target hardware has 7 GB RAM, a 2012-era CPU, and a 3 GB GPU with
no modern compute stack. This is not a limitation to work around — it
is a design constraint that shapes the architecture.

Every component must justify its memory footprint. The runtime itself
should use under 100 MB. Model inference is the largest consumer and
must be isolated so it can be loaded, unloaded, and swapped without
affecting the rest of the system.

**Implication:** Rust, not Python. Single binary, not virtual
environments. Trait-based model isolation, not deep coupling.

## 3. Code-awareness is first-class

Code is not just text to be searched. Code has structure: functions
call functions, classes extend classes, modules import modules.
Observations and rules are *about* specific code structures. This
relationship is the core of "code-aware cognition."

The graph must connect knowledge entities to code entities with
explicit edges. Retrieval must traverse these edges. When code
changes, the system must know which knowledge might be affected.

**Implication:** Tree-sitter indexing produces first-class entities in
the same graph as observations and rules. Knowledge-to-code edges are
not metadata — they are the graph.

## 4. Context assembly is the product

Agents do not want search results. They want context — a coherent,
scoped, ranked collection of information that helps them do their
current task. Search is a means to that end.

The primary output of CogZ is a context pack. Every other feature
(storage, search, graph traversal, consolidation) exists to produce
better context packs. If a feature does not improve context quality,
it does not belong in the system.

**Implication:** Context packs include provenance (graph paths showing
why each piece was included). The context assembly module is the
center of the architecture, not a peripheral feature.

## 5. Simplicity over completeness

Fewer features, deeper integration. A system with 13 well-integrated
tools is more useful than one with 40 superficial ones.

Every feature must answer: does this improve the agent's memory or
context? If the answer is "it might be useful someday," the feature
does not ship. Scope creep is the primary cause of architectural
debt. CogZ-py had ~40 MCP tools, half of which were never used. CogZ
ships 13 and makes each one count.

**Implication:** No speculative features. No "this might be useful
later." If it's not needed now, it's not built.

## 6. Models are interchangeable

The system must work with any embedding model, any NLI model, any
reranker — or none at all. Models are consumed via traits. The
storage, search, and consolidation layers never import model-specific
code.

Swapping a model is a configuration change, not a code change. If the
embedding model is unavailable, the system degrades gracefully —
FTS-only search, no contradiction detection, title-based dedup only
(no embedding similarity) — but continues to function.

**Implication:** Model traits define the interface. Implementations
are separate modules. The runtime wires them based on config, not
hardcoded imports.

## 7. Knowledge is a graph

All entities — observations, rules, knowledge entries, functions,
classes, files, modules — are nodes in a single graph. Relationships
are edges. There are no separate tables for separate types. There is
one entity table, one edge table, one FTS index, one embedding table.

The graph is the source of truth. Typed views are derived projections,
not separate stores. This eliminates dual storage, double write paths,
and FTS duplication.

**Implication:** One schema, one ID space, one graph. Type-specific
fields live in JSON properties, not in separate columns.

## 8. Consolidation is continuous

Memory degrades without maintenance. Duplicates accumulate,
contradictions go undetected, stale knowledge persists. Batch
consolidation that "runs later" means consolidation that rarely runs.

Consolidation happens on every insert. Dedup checks against existing
entries in the same scope. Contradiction detection runs against
related entries. Promotion and merge happen as lightweight background
operations. No 7-phase batch pipeline.

**Implication:** Insert path includes consolidation hooks. Background
processing is for expensive operations (embedding, NLI), not for
basic dedup.

## 9. Files are canonical, the database is derived

All entities are markdown files on disk. The database is a derived
index — embeddings, FTS, graph edges — rebuilt from files and source
code. There is no DB-canonical content. `cogz reindex` rebuilds
everything.

Every write touches a file first, then syncs the DB. If the file
write fails, the DB is not updated. No inconsistency is possible.
Sync is one-directional: file → DB. There is no bidirectional sync,
no conflict resolution, no "which is newer?" problem.

Git tracks human-readable canonical content (knowledge, rules,
config). Everything derived or transient is gitignored (observations,
database). A fresh clone gets curated knowledge and rules; `cogz
index` rebuilds the DB from files + code.

**Implication:** The DB is disposable. Files survive DB corruption.
Knowledge and rules are git-trackable, portable, reviewable. Code
indexing respects `.gitignore`; `.cogz/` content sync is git-agnostic
(all files on disk are indexed regardless of git status).

## 10. Bounded growth

The system must not grow unboundedly with usage. The events table must
not bloat. The embedding table must not accumulate stale vectors. The
knowledge directory must not fill with duplicates.

Every store has a retention policy or a compaction mechanism. Indexing
operations do not produce domain events. Superseded entities are
prunable. The system is designed to run for months without manual
maintenance.

**Implication:** No event sourcing for structural operations. Domain
events only. Compaction is built in, not bolted on.
