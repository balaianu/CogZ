# Entity Model

CogZ stores three types of file-backed entities (observations, rules, knowledge) and four types of code entities (functions, classes, files, modules). This document describes the file format, frontmatter schema, and state machine.

## File-backed entities

### File format

Every entity is a Markdown file with YAML frontmatter:

```markdown
---
id: 12345678-1234-1234-1234-123456789012
title: Auth middleware checks JWT expiry
type: observation
status: active
created_at: 2026-09-07T09:00:00Z
updated_at: 2026-09-07T09:00:00Z
references: []
---

The auth middleware checks JWT expiry before hitting the route handler.
```

The frontmatter parser is a hand-rolled YAML subset (`src/files/frontmatter.rs`) that handles only scalar key-value pairs and inline string arrays. No nested mappings, no anchors, no multi-line strings. This avoids a full YAML dependency for what is a very constrained format.

### Common frontmatter fields

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | UUID string | yes | Entity UUID. File-backed entities use v4; must be a valid UUID. |
| `title` | string | yes | Short title. |
| `type` | string | yes | `observation`, `rule`, or `knowledge`. |
| `status` | string | yes | Entity status (see state machine below). |
| `created_at` | RFC 3339 string | yes | Creation timestamp. |
| `updated_at` | RFC 3339 string | yes | Last modification timestamp. |
| `references` | array of strings | yes | UUIDs this entity references. Can be empty. |

### Type-specific fields

**Observations:**
- `source` (string, optional) — who or what produced this observation. Default: `"agent"`.
- `supporting_ids` (array of strings, optional) — UUIDs of observations this observation supports.

**Rules:**
- `confidence` (float, optional) — confidence score (0.0–1.0).
- `derived_from` (UUID string, optional) — set by promotion when an observation is promoted to a rule.

**Knowledge:**
- `category` (string, required) — becomes a subdirectory under `knowledge/`. Sanitized to a slug.
- `tags` (array of strings, optional) — tags for filtering.

### File paths

Entity files are stored under `.cogz/` with this structure:

```
.cogz/
  knowledge/
    <category>/
      <slug>-<hash>.md
  rules/
    <slug>.md
  observations/
    <YYYY-MM>/
      <slug>-<hash>.md
```

Slugs are generated from the title: lowercase, spaces → hyphens, non-alphanumeric stripped, max 60 chars (breaking on hyphen boundaries). A short hash suffix is appended to avoid collisions.

### State machine

```
active    → stale | rejected | superseded
stale     → active
rejected  → pruned
superseded→ pruned
pruned    → (terminal)
```

| Status | Meaning |
|---|---|
| `active` | Live, in use. Default on creation. |
| `stale` | Code it referenced has changed. Can be re-activated if the code reverts or the entity is updated. |
| `rejected` | Manually rejected. Eligible for pruning. |
| `superseded` | Replaced by another entity (merge). Has a `superseded_by` field. Eligible for pruning. |
| `pruned` | Terminal. Content removed, embedding removed, FTS entry removed. Graph edges preserved. |

Illegal transitions are rejected by `transition_status()` with an `IllegalTransition` error.

### Update policy

- **Observations are append-only for content.** No `update_observation` tool exists. The body must not change after creation — `cogz doctor` detects content edits via content hash comparison.
- **Rules are append-only for substantive changes.** To change a rule, supersede it and create a new one. Status changes (frontmatter only) are allowed in-place.
- **Knowledge is the only entity type with in-place content edits.** The `update_knowledge` tool overwrites the file and re-syncs the DB.

### UUIDs

- **File-backed entities** get random UUID v4 IDs on creation.
- **Code entities** get deterministic UUID v5 IDs derived from `{file_path}:{entity_type}:{qualified_name}`. This ensures `cogz reset` + `cogz index` produces identical IDs, preserving `references` edges from observations.
- **Code entity paths are normalized** to forward slashes regardless of platform, so the same source file produces the same UUID on Linux, macOS, and Windows.

## Code entities

Code entities are extracted from source code by tree-sitter. They are not file-backed — they exist only in the DB.

| Type | Extracted from | Example |
|---|---|---|
| `function` | Function definitions | `fn authenticate()` in Rust |
| `class` | Class/struct/impl definitions | `struct User` in Rust |
| `file` | Source files | `src/middleware/auth.rs` |
| `module` | Module declarations | `mod auth;` in Rust |

**Supported languages:** Rust, Python, Go, JavaScript, TypeScript, TSX, Bash.

**Structural edges** (extracted from ASTs):

| Edge | From → To | Example |
|---|---|---|
| `calls` | function → function | `authenticate()` calls `verify_jwt()` |
| `imports` | module/file → module/file | `use crate::auth;` |
| `extends` | class → class | `impl Trait for Foo` |
| `contains` | file/module → function/class/module | `auth.rs` contains `authenticate()` |

**Auto-links:** Knowledge entities are automatically linked to code entities by scanning content for file paths and unique symbol names. These are `auto_references` edges — DB-only, not in frontmatter, and fully rebuildable on reindex.

**Test file exclusion:** Test files (conventions: `tests/` dir, `*_tests.rs`, `*_test.go`, `test_*.py`, `*.test.ts`, etc.) are excluded from search results and context packs by default. Set `include_tests: true` in `SearchParams` to include them.

## Pruning

Pruning removes content from `rejected` and `superseded` observations older than `retention.observation_prune_after_days` (default: 90 days).

- Only `rejected` and `superseded` observations are prunable. Active and stale observations are never pruned.
- Rules and knowledge are never pruned.
- Pruning is never automatic. Requires explicit `cogz doctor --prune-observations --confirm`.
- Pruned entities become tombstones: `status='pruned'`, empty content, no embedding, no FTS entry. Graph edges are preserved.
- `pruned` is terminal — no transitions out.

## See also

- [Architecture](architecture.md) — system overview and module map
- [Schema](../dev/schema.md) — DB schema and migrations
- [CLI Reference](../cli-reference.md) — `cogz doctor --prune-observations`
