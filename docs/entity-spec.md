# CogZ — Entity File Specification

This document defines the canonical format for all entity files.
Every entity (observation, rule, knowledge) is a markdown file with
YAML frontmatter. Code entities (function, class, file, module) are
not files — they're extracted from source code by the indexer.

---

## File Format

All entity files share the same structure:

```markdown
---
<frontmatter>
---

<markdown content>
```

The frontmatter is YAML between `---` delimiters. The content is
markdown, rendered as-is in search results and context packs.

---

## Frontmatter Schema

### Common fields (all entity types)

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string (UUID v4) | Yes | Unique identifier. Generated on creation, never changed. |
| `title` | string | Yes | Human-readable title. Used in search results, context packs, and file naming. |
| `type` | string | Yes | Entity type: `observation`, `rule`, `knowledge`. |
| `status` | string | Yes | One of: `active`, `stale`, `superseded`, `rejected`, `pruned`. Default on creation: `active`. `pruned` is set only by `cogz doctor --prune-observations` on tombstoned entities. |
| `created_at` | string (ISO 8601) | Yes | Creation timestamp. |
| `updated_at` | string (ISO 8601) | Yes | Last modification timestamp. |
| `references` | array of strings (UUIDs) | No | Entity UUIDs this entity references. Creates `references` graph edges. |

### Type-specific fields

#### Observation

| Field | Type | Required | Description |
|---|---|---|---|
| `source` | string | No | Who produced this: `agent` (default), `human`, `hook`. |
| `confidence` | number (0.0-1.0) | No | Confidence score. Default: 0.5 for raw observations. |
| `supporting_ids` | array of strings (UUIDs) | No | Entity UUIDs of observations that support this one. Used for promotion. |

#### Rule

| Field | Type | Required | Description |
|---|---|---|---|
| `confidence` | number (0.0-1.0) | No | Confidence score. Default: 1.0 for directly created rules, lower for promoted. |
| `validation_count` | integer | No | Number of times this rule has been validated. Default: 0. |
| `supporting_ids` | array of strings (UUIDs) | No | Entity UUIDs of observations that support this rule. |
| `promoted_from` | string (UUID) | No | Observation UUID this rule was promoted from. Only set by consolidation. |
| `superseded_by` | string (UUID) | No | Entity UUID of the rule that superseded this one. Set when status transitions to `superseded`. |

#### Knowledge

| Field | Type | Required | Description |
|---|---|---|---|
| `category` | string | Yes | Category for organization. Determines subdirectory. Examples: `architecture`, `decisions`, `patterns`, `gotchas`. |
| `tags` | array of strings | No | Free-form tags for additional organization. |

---

## File Naming Convention

### Knowledge files

```
.cogz/knowledge/<category>/<slug>.md
```

- `category` is from the frontmatter (lowercase, hyphenated)
- `slug` is derived from the title: lowercase, spaces → hyphens,
  non-alphanumeric stripped, max 60 chars
- Examples:
  - `.cogz/knowledge/architecture/search-design.md`
  - `.cogz/knowledge/decisions/use-sqlite-over-postgres.md`
  - `.cogz/knowledge/gotchas/fts5-ranking-with-rrf.md`

If a slug collision occurs (two knowledge entries with the same
title in the same category), a short hash suffix is appended:
`search-design-a3f2.md`.

### Rule files

```
.cogz/rules/<slug>.md
```

- Same slug derivation as knowledge
- No subdirectories — rules are flat
- Examples:
  - `.cogz/rules/use-parameterized-queries-for-fts5.md`
  - `.cogz/rules/validate-input-before-search.md`

### Observation files

```
.cogz/observations/<year-month>/<uuid>.md
```

- Organized by year-month for chronological browsing
- Named by UUID, not slug — observations are numerous, auto-generated,
  and not meant to be browsed by title
- Examples:
  - `.cogz/observations/2026-08/550e8400-e29b-41d4-a716-446655440000.md`
  - `.cogz/observations/2026-08/6ba7b810-9dad-11d1-80b4-00c04fd430c8.md`

---

## File Examples

### Observation

```markdown
---
id: 550e8400-e29b-41d4-a716-446655440000
title: "FTS5 ranking bug in _build_fts_search_sql"
type: observation
status: active
created_at: 2026-08-27T14:30:00Z
updated_at: 2026-08-27T14:30:00Z
references: ["a1b2c3d4-e5f6-4789-abcd-000000000017"]
source: agent
confidence: 0.7
---

The RRF fusion in _build_fts_search_sql produces incorrect rankings
when k=60 and the FTS result set is smaller than the vector result
set. The issue is in the score normalization step — FTS scores are
not scaled to the same range as vector distances before fusion.
```

### Rule

```markdown
---
id: 6ba7b810-9dad-11d1-80b4-00c04fd430c8
title: "Use parameterized queries for FTS5"
type: rule
status: active
created_at: 2026-08-27T15:00:00Z
updated_at: 2026-08-27T15:00:00Z
references: ["a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023"]
confidence: 1.0
validation_count: 2
supporting_ids: ["550e8400-e29b-41d4-a716-446655440042", "550e8400-e29b-41d4-a716-446655440043"]
---

Always use parameterized queries when constructing FTS5 search SQL.
Never interpolate user input into the query string — this is both a
security (SQL injection) and correctness (FTS5 syntax errors) concern.
```

### Knowledge

```markdown
---
id: f47ac10b-58cc-4372-a567-0e02b2c3d479
title: "Search Architecture"
type: knowledge
status: active
created_at: 2026-08-27T09:00:00Z
updated_at: 2026-08-27T09:00:00Z
references: ["a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023", "a1b2c3d4-e5f6-4789-abcd-000000000045"]
category: architecture
tags: [search, fts5, vector, rrf]
---

# Search Architecture

The search system uses hybrid FTS5 + vector search with RRF fusion.

## Components

- **FTS5**: Full-text search over entity titles and content
- **Vector**: Cosine similarity over embeddings (CodeRankEmbed for
  code, bge-base for knowledge)
- **RRF**: Reciprocal Rank Fusion merges FTS and vector results

## Flow

1. Query is run against both FTS5 and vec0
2. Results are fused using RRF with k=60
3. Graph expansion follows edges from top results
4. Expanded results include graph paths for provenance
```

---

## Code Entities (Not Files)

Code entities (function, class, file, module) are not stored as
markdown files. They are extracted from source code by tree-sitter
and exist only in the database. Source code is their canonical store.

### Code entity properties (in DB `properties` JSON)

#### Function

```json
{
  "file_path": "src/cogz/storage/crud.rs",
  "line_start": 45,
  "line_end": 82,
  "language": "rust",
  "signature": "pub fn insert_entity(conn: &Connection, entity: &Entity) -> Result<i64>",
  "qualified_name": "cogz::storage::crud::insert_entity"
}
```

#### Class

```json
{
  "file_path": "src/cogz/search/hybrid.rs",
  "line_start": 12,
  "line_end": 156,
  "language": "rust",
  "qualified_name": "cogz::search::hybrid::HybridSearch",
  "kind": "struct_item"
}
```

For Rust `impl` blocks, the `qualified_name` includes an `impl` prefix
(e.g. `impl Point` or `impl Display for Point`) to distinguish from
the struct/trait entity with the same type name. The `kind` property
is `impl`, and the `trait` property holds the trait name when present.

For Python classes, the `superclasses` property lists parent class
names (used for `extends` edge extraction):

```json
{
  "file_path": "src/models.py",
  "line_start": 5,
  "line_end": 20,
  "language": "python",
  "qualified_name": "Dog",
  "superclasses": ["Animal"]
}
```

#### File

```json
{
  "file_path": "src/cogz/storage/crud.rs",
  "language": "rust",
  "line_count": 287
}
```

#### Module

```json
{
  "file_path": "src/cogz/storage/mod.rs",
  "language": "rust",
  "module_path": "cogz::storage"
}
```

### Code entity content

The `content` field for code entities is the source code text of the
entity:

- **Function**: the full function body (signature through closing brace)
- **Class**: the full class/struct/impl block
- **File**: the full file content
- **Module**: the module declaration and its doc comment (if any)

This content is what gets embedded (with CodeRankEmbed) and indexed
in FTS5. When a search returns a code entity, the agent sees the
actual source code.

---

## References and Graph Edges

### How references work

The `references` field in frontmatter creates `references` graph
edges from the entity to the listed entity IDs:

```yaml
references: ["a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023"]
```

This creates:
- Edge: this entity → entity `a1b2c3d4-...0017` (type: `references`)
- Edge: this entity → entity `a1b2c3d4-...0023` (type: `references`)

Entity IDs are UUID v4 strings, generated on creation and stored in
the file's `id` field. They are stable across DB rebuilds — the same
UUID in the file always maps to the same entity. Code entities (no
file on disk) get deterministic UUID v5 values derived from
`{file_path}:{entity_type}:{qualified_name}`, ensuring the same code
entity gets the same UUID across rebuilds.

### Referencing code entities

When an agent records an observation about a function, it passes the
function's entity UUID:

```
Agent calls: record_observation(content="...", references=["a1b2c3d4-e5f6-4789-abcd-000000000017"])
  where a1b2c3d4-...0017 is the UUID of the function _build_fts_search_sql
```

The agent discovers code entity UUIDs via `search` or `list_entities`.
The `references` field in the written file stores these UUIDs.

### Edge types

| Edge type | Source | Target | Who creates |
|---|---|---|---|
| `references` | observation/rule/knowledge | any entity | Agent (via frontmatter) |
| `calls` | function | function | Indexer (tree-sitter) |
| `imports` | module/file | module/file | Indexer (tree-sitter) |
| `extends` | class | class | Indexer (tree-sitter) |
| `supports` | observation | observation/rule | Consolidation (automatic) |
| `contradicts` | observation/rule | observation/rule | Consolidation (NLI-based) |
| `derived_from` | rule | rule/observation | Consolidation (on promotion) or agent (on supersede) |

---

## Content Hash and Change Detection

Each entity in the DB has a `content_hash` field — a SHA-256 hash of
the file's content (frontmatter + markdown body).

On `cogz reindex`:
1. Read file from disk
2. Compute content hash
3. Compare with DB `content_hash`
4. If different: re-embed, update FTS, update DB entity
5. If same: skip (no unnecessary work)

This makes incremental reindexing efficient — only changed files are
re-processed.

---

## File Sync State Machine

```
                    ┌─────────┐
                    │  FILE   │
                    │ EXISTS  │
                    └────┬────┘
                         │
              ┌──────────┼──────────┐
              │          │          │
         hash =    hash ≠      file not
         DB hash   DB hash    found
              │          │          │
              ▼          ▼          ▼
         SKIP     RE-EMBED    MARK STALE
                  + UPDATE    (don't delete
                  DB ENTITY    — preserve
                               edges)
```

Deleted files are marked `status = 'stale'`, not removed from the DB.
This preserves graph edges for audit — you can see that an observation
once referenced a function even after the knowledge file was deleted.
A `cogz doctor` check can report stale entities and offer to purge
them if desired.

---

## Update Policy

The DB is the cognition layer. It can only reason about what it has.
Git history is invisible to the cognition engine. If a file is edited
in place, the DB updates and the old version is gone from the DB's
perspective. This creates the key split: **content updates vs. status
updates**, and **git-tracked vs. gitignored**.

### Per entity type

| Entity | Content update | Status update | Why |
|---|---|---|---|
| Knowledge | In-place | In-place | Documentation — current version matters, git preserves history |
| Rule (minor edit) | In-place | In-place | Wording fix, directive unchanged |
| Rule (substantive) | Supersede + new | In-place | Directive changed — DB needs both versions for cognitive continuity |
| Observation | Append-only (new observation) | In-place | Raw experience — corrections are new observations, not rewrites |

**Unifying principle:** memory is additive, with state transitions.
Destructive edits are only allowed when git preserves the history
(knowledge, rules) or when the edit is metadata, not content (status
changes).

### Knowledge: in-place updates

Knowledge is documentation. The current version is what matters. You
edit a wiki page, you don't create a new page and supersede the old
one every time. Git-tracked → `git log` preserves every version for
human reference. The DB only needs the current, accurate
documentation. Forcing append-only would be unbearable — you'd have
50 versions of "Search Architecture".

### Rules: minor edits vs. substantive changes

**Minor edits** (typo, wording clarification, adding an example):
in-place. Git preserves the edit. The directive itself didn't change.

**Substantive changes** (the directive itself changes): create a new
rule, mark the old one `status = superseded`, set
`superseded_by` in old rule's frontmatter, add a `derived_from` edge
from new → old. The DB now has both:

```
Old rule: status=superseded, properties.superseded_by="6ba7b810-9dad-11d1-80b4-00c04fd43056"
New rule: id="6ba7b810-9dad-11d1-80b4-00c04fd43056", status=active, edge: derived_from → old rule
```

The agent can query superseded rules, see the history, understand the
transition. This preserves cognitive continuity — the DB knows that
behavior changed and why.

### Observations: append-only for content

Observations are raw experience. Rewriting experience is dishonest —
you didn't originally observe the corrected version. The correction
is itself a new observation.

**Content:** never edit. If an observation was wrong, create a new
observation that contradicts it:

```
Observation 550e8400-...0042: "Bug is in _build_fts_search_sql" (status: active)
Observation 550e8400-...0043: "Bug is actually in _fts_search_all, not _build_fts_search_sql"
    edge: contradicts → 550e8400-...0042
```

Consolidation then resolves it: 550e8400-...0042 gets `status = rejected`,
550e8400-...0043 stays active.

**Status:** in-place edits allowed. When code changes, observations
referencing that code get `status = stale`. This is a lifecycle
transition, not a content change — the observation still says what it
always said, it's just no longer reliable.

Observations are gitignored — no git history to recover the original.
This reinforces the append-only policy: there is no safety net if
content is overwritten.

---

## Status State Machine

Status transitions go through a validated state machine, enforced in
the storage layer. Whether triggered by consolidation or by
frontmatter edits, the transition must be legal.

```
                    ┌─────────┐
          ┌─────────│ active  │─────────┐
          │         └────┬────┘         │
          │              │              │
          │     ┌────────┼────────┐     │
          │     │        │        │     │
          ▼     ▼        ▼        ▼     ▼
     ┌───────┐ ┌──────┐ ┌──────────┐
     │ stale │ │reject│ │superseded│
     └───┬───┘ └──┬───┘ └────┬─────┘
         │        │          │
         ▼        ▼          ▼
     back to   terminal   terminal
     active    (create    (create
     (code     new obs)   new rule)
     reverted)
```

| From | To | Trigger |
|---|---|---|
| `active` | `stale` | Code changed, observation/rule may be outdated |
| `active` | `rejected` | Contradicted by another observation (NLI or manual) |
| `active` | `superseded` | Replaced by a newer rule (derived_from edge exists) |
| `stale` | `active` | Code reverted or observation re-validated |
| `rejected` | `pruned` | `cogz doctor --prune-observations` (tombstone replaces entity) |
| `superseded` | `pruned` | `cogz doctor --prune-observations` (tombstone replaces entity) |
| `rejected` | *(none else)* | Terminal — create a new observation instead |
| `superseded` | *(none else)* | Terminal — create a new rule instead |
| `pruned` | *(none)* | Terminal — entity is a tombstone, graph edges preserved |

Rejected and superseded are terminal states. You cannot revive them.
This is intentional — if a rule was superseded, the new rule is the
current truth. If you want to go back, create another new rule that
supersedes the superseder. The history is preserved in the graph.

### Enforcement in code

```rust
fn transition_status(current: &str, next: &str) -> Result<()> {
    let allowed = match current {
        "active" => ["stale", "rejected", "superseded"].as_slice(),
        "stale" => ["active"].as_slice(),
        "rejected" => ["pruned"].as_slice(),  // only via doctor prune
        "superseded" => ["pruned"].as_slice(), // only via doctor prune
        "pruned" => &[],  // terminal (tombstone)
        _ => return Err(invalid_status(current)),
    };
    if !allowed.contains(&next) {
        return Err(illegal_transition(current, next));
    }
    Ok(())
}
```

### Consolidation as the sanctioned edit path

Consolidation is the only code that modifies entity status. It does
so by editing the file's frontmatter (not the content body), then
syncing:

```
Consolidation decides: observation 550e8400-...0042 should be rejected
    │
    ▼
Read file .cogz/observations/2026-08/<uuid>.md
    │
    ▼
Parse frontmatter
    │
    ▼
Change status: active → rejected
    (content body unchanged)
    │
    ▼
Write file back (frontmatter updated, body untouched)
    │
    ▼
Sync DB (file → DB)
    │
    ▼
Record domain event: observation_rejected
```

This respects "everything is a file" — the status change is in the
file, not just the DB. And it respects "content is append-only for
observations" — only the frontmatter changes, the body is untouched.

---

## Enforcement Layers

There are two write paths: agent-initiated (MCP/CLI tools) and
human-initiated (direct file edits). We can refuse the first, but
only detect the second.

### Layer 1: MCP tool surface — no edit tools for observations and rules

The MCP contract has no `update_observation` or `edit_rule` tool. The
agent can only create new observations and rules. If an observation
is wrong, the agent creates a new observation with a `contradicts`
edge. If a rule needs to change, the agent creates a new rule with a
`derived_from` edge to the old one, and the old one gets superseded.

**This is the primary enforcement. You can't do the wrong thing
because the tool doesn't exist.**

For knowledge, `update_knowledge` takes an entity ID and new content,
overwrites the file, then syncs. Knowledge is documentation —
in-place edits are the correct workflow.

### Layer 2: File sync — detect and handle content changes by entity type

When `cogz reindex` finds a content hash mismatch (file changed since
last sync), the sync layer applies policy based on entity type:

**Observations:** The sync layer updates the DB entity to match the
file (files are canonical — we can't pretend the file didn't change).
The old content is recorded in the event log as an
`observation_edited` domain event (audit trail). `cogz doctor`
reports this as a policy violation.

**Rules:** If only frontmatter changed (status, confidence,
validation_count) → update DB, this is a sanctioned lifecycle change.
If content body changed → log warning, update DB to match file,
record `rule_edited` event with old content. `cogz doctor` reports
content edits on rules.

**Knowledge:** Content change detected → update DB. This is normal
and expected.

### Layer 3: Status state machine — validated transitions

All status transitions go through `transition_status()`. Illegal
transitions (e.g., `rejected → active`) are rejected with an error.
This is enforced in the storage layer regardless of who initiated
the change.

### Layer 4: Doctor — detect human violations

`cogz doctor` checks for policy violations that bypassed the tool
surface:

- **Observation content edited:** DB has old content hash, file has
  new hash, entity type is observation → report
- **Rule content edited substantively:** same check for rules
- **Illegal status transition:** DB shows status went from `rejected`
  to `active` without a new entity → report
- **Orphaned supersede:** rule marked `superseded` but no
  `derived_from` edge to a new rule → report
- **Missing file:** DB entity exists but file is gone and not marked
  stale → report

Doctor doesn't fix these automatically — it reports them. The human
decides what to do.

---

## Duplicate Detection

Duplicates happen — a new session, the agent doesn't know a knowledge
entry already exists, creates another one. Same content, different
file, different entity ID. The system detects and surfaces these.

### Detection stack

| Check | When | Cost | What it catches |
|---|---|---|---|
| Exact title match | On create | Negligible | Same title, almost certainly duplicate |
| Fuzzy title match | On create | Low | Similar titles, likely related |
| Embedding similarity > 0.92 | On create (dedup hook) | One embedding + KNN query | Semantic duplicates |
| Embedding similarity > 0.80 | On `cogz doctor` | Pairwise, expensive | Near-duplicates that slipped through |

### On-insert behavior

Every `create_knowledge`, `create_rule`, and `record_observation`
triggers the dedup hook after embedding:

```
create entity → write file → sync DB → embed content
    │
    ▼
Dedup check: compare embedding against existing entities of same type
    │
    ▼
Similarity > dedup_threshold (0.92)?
    │
    ├── No → normal completion
    │
    └── Yes → flag as potential duplicate
              │
              ▼
         Return warning to caller with existing entity ID,
         similarity score, and suggestion
```

**Knowledge duplicates are flagged, not auto-resolved.** Knowledge is
human-curated documentation. The system shouldn't silently merge two
architecture docs — they might cover different aspects, or the new
one might be a deliberate rewrite.

**Observation duplicates are flagged and feed into consolidation.**
Consolidation can auto-merge confirmed duplicates (similarity >
threshold + same references) since observations are raw experience,
not curated content.

**Rule duplicates are flagged.** Two rules with the same directive
are noise, but auto-merging rules is risky — they might have
different confidence levels or supporting observations. Consolidation
surfaces these for review.

### Title-based detection

Embedding similarity catches semantic duplicates. But there's a
cheaper signal: title similarity. Two knowledge entries titled
"Search Architecture" and "Search Architecture" (or "Search Arch" and
"Search Architecture") are almost certainly duplicates or related.

```
On create:
    │
    ▼
Check: does an entity with the same or similar title already exist?
    │
    ├── Exact title match → strong warning (almost certainly a duplicate)
    └── Fuzzy title match (Levenshtein distance < threshold) → soft warning
```

This is cheap (title comparison, not embedding) and catches the most
common case: the agent or human uses the same title without checking.

### Doctor periodic check

`cogz doctor` runs a broader pairwise similarity check at a lower
threshold (0.80) to catch near-duplicates that slipped through the
on-insert check:

```
cogz doctor
    │
    ▼
For all knowledge entities, compute pairwise similarity
    │
    ▼
Report pairs with similarity > 0.80
    │
    ▼
"Potential near-duplicate knowledge entries:
      f47ac10b-...d479 'Search Architecture' ↔ f47ac10b-...d461 'Search Design' (similarity: 0.87)
     Not auto-flagged (below 0.92 threshold). Review manually."
```

This is a periodic check, not an on-insert check. It's expensive
(pairwise comparison) so it runs in `doctor`, not on every write.

---

## Frontmatter Validation

On file sync, the frontmatter is validated:

1. **Required fields present**: `id`, `title`, `type`, `status`,
   `created_at`, `updated_at`
2. **Type-specific fields present**: `category` for knowledge, etc.
3. **Field types correct**: dates are ISO 8601, `references` is an
   array of UUID strings, etc.
4. **Referenced entity UUIDs exist**: each UUID in `references` must
   correspond to an entity in the DB

If validation fails:
- The file is not synced (DB entity not created/updated)
- A warning is logged
- `cogz doctor` reports the invalid file

This prevents malformed files from corrupting the DB or graph.

---

## Future: Provenance for Agent-Written Knowledge

The `source` field currently exists only on observations. Knowledge
and rules have no provenance field — every entry is implicitly trusted
equally.

This is fine while a human reviews every write. It stops being fine
when agents write knowledge that other agents consume without a human
in between. The roadmap has two such transitions:

- **Phase 9 (consolidation):** auto-promotion turns agent observations
  into rules/knowledge. The promoted entity has no human review.
- **Phase 11 (events/hooks):** long-running autonomous sessions write
  knowledge without per-write human review.

At that point, extend `source` to knowledge and rules with the same
values: `agent`, `human`, `hook`. Surface it in context packs and
search results as visible metadata — no gating, no blocking. The
consuming agent or human weights it themselves.

This is provenance transparency, not a review queue. A heavier
mechanism (a `reviewed` boolean, a `needs_review` status, a
`cogz doctor --review` workflow) only makes sense if agents are
empirically writing wrong knowledge that other agents act on blindly.
Wait for evidence before building the machinery.
