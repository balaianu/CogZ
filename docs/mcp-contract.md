# CogZ — MCP Tool Contract

This document defines the exact interface of every MCP tool CogZ
exposes. Each tool has a precise signature, parameter types, return
shape, and error behavior. This is the contract the agent depends on.

---

## Design Rules

- **Verb-first naming:** `<verb>_<noun>`, flat namespace
- **`get_`** returns a singleton or summary (no filtering)
- **`query_`** returns a filtered collection (parameters select a subset)
- **`list_`** enumerates all items of a kind (no filtering)
- **`record_` / `create_`** inserts a new entity (writes file, syncs DB)
- **`update_`** edits an existing entity in-place (only knowledge — observations and rules are append-only)
- **All writes go through files first** — no tool writes directly to DB
- **Errors are structured** — MCP error response with a clear message
- **All tools are synchronous from the agent's perspective** —
  background operations (embedding, consolidation) happen after the
  tool returns
- **Every tool requires an explicit `repo` parameter** — absolute path
  to the project root containing `.cogz/`. No fallback to cwd, no
  implicit session context. This aligns with the 2026-07-28 MCP spec
  (SEP-2577) which deprecated Roots in favor of tool parameters. The
  `repo` parameter is omitted from individual tool schemas below for
  brevity but is required on every tool.

---

## Tools

### 1. `record_observation`

Record a raw observation about the codebase.

```json
{
  "name": "record_observation",
  "description": "Record an observation about the codebase. Observations are raw, unvalidated experience — bugs found, decisions made, patterns noticed. They persist across sessions and can be promoted to rules through consolidation.",
  "inputSchema": {
    "type": "object",
    "required": ["content"],
    "properties": {
      "content": {
        "type": "string",
        "description": "The observation text. Should be specific and actionable."
      },
      "title": {
        "type": "string",
        "description": "Short title for the observation. Auto-generated from content if omitted."
      },
      "references": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Entity UUIDs of code entities (functions, classes, files) this observation is about. Creates 'references' graph edges."
      },
      "source": {
        "type": "string",
        "description": "Who or what produced this observation. Default: 'agent'.",
        "default": "agent"
      }
    }
  }
}
```

Note: `confidence` is not a parameter. Observations start at 0.5
(default). Confidence is adjusted by consolidation (supporting
observations increase it, contradictions decrease it). The agent
does not self-assess confidence — the system derives it from evidence.

**Returns:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440042",
  "file_path": ".cogz/observations/2026-08/<uuid>.md",
  "status": "active",
  "dedup_flagged": false,
  "contradiction_flagged": false,
  "duplicate_warning": null
}
```

`duplicate_warning` is `null` when no duplicate detected, or:
```json
{
  "existing_id": "550e8400-e29b-41d4-a716-446655440040",
  "existing_title": "FTS5 ranking issue in search",
  "similarity": 0.94,
  "title_match": "fuzzy",
  "suggestion": "This observation may duplicate existing observation 550e8400-...0040. If correcting it, create a contradicts edge instead. If distinct, ignore this warning."
}
```

**Side effects:**
- Writes markdown file to `.cogz/observations/<year-month>/<uuid>.md`
- Syncs to DB (entity + embedding + FTS)
- Runs dedup check (sets `dedup_flagged` if similar observation exists)
- Runs title match check (sets `duplicate_warning` if title matches existing)
- Runs contradiction check (sets `contradiction_flagged` if NLI detects contradiction)
- Records `observation_created` domain event

**Errors:**
- File write fails → MCP error, no DB change
- Embedding model unavailable → success with no vector (graceful degradation, logged)

---

### 2. `query_observations`

Query observations by various filters.

```json
{
  "name": "query_observations",
  "description": "Query observations with optional filters. Returns matching observations sorted by recency.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "status": {
        "type": "string",
        "enum": ["active", "stale", "superseded", "rejected", "pruned"],
        "description": "Filter by status. Default: 'active'.",
        "default": "active"
      },
      "references": {
        "type": "string",
        "description": "Filter to observations referencing this entity UUID."
      },
      "limit": {
        "type": "integer",
        "description": "Max results. Default: 20.",
        "default": 20
      }
    }
  }
}
```

**Returns:**
```json
{
  "observations": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440042",
      "title": "FTS5 ranking bug in _build_fts_search_sql",
      "content": "The RRF fusion produces incorrect rankings when k=60...",
      "status": "active",
      "references": ["a1b2c3d4-e5f6-4789-abcd-000000000017"],
      "source": "agent",
      "created_at": "2026-08-27T14:30:00Z",
      "updated_at": "2026-08-27T14:30:00Z",
      "file_path": ".cogz/observations/2026-08/<uuid>.md"
    }
  ],
  "count": 1
}
```

Note: Query tools return core fields only. Type-specific properties
(`confidence`, `supporting_ids`, `validation_count`, `promoted_from`,
`superseded_by`) are omitted from query results to keep responses
compact. Use `search` or `get_context` for full content, or read the
file directly via `file_path`. Entity IDs are UUID strings throughout.

---

### 3. `create_rule`

Create a validated rule. Rules are trustworthy directives the agent
should follow.

```json
{
  "name": "create_rule",
  "description": "Create a rule — a validated directive the agent should follow. Rules are git-tracked and shared. Use for coding standards, design decisions, and confirmed patterns.",
  "inputSchema": {
    "type": "object",
    "required": ["content"],
    "properties": {
      "content": {
        "type": "string",
        "description": "The rule text. Should be a clear, actionable directive."
      },
      "title": {
        "type": "string",
        "description": "Short title. Auto-generated from content if omitted."
      },
      "references": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Entity UUIDs of code entities this rule applies to."
      },
      "confidence": {
        "type": "number",
        "description": "Confidence score 0.0-1.0. Default: 1.0 for directly created rules.",
        "default": 1.0
      }
    }
  }
}
```

**Returns:**
```json
{
  "id": "6ba7b810-9dad-11d1-80b4-00c04fd43055",
  "file_path": ".cogz/rules/<slug>.md",
  "status": "active",
  "dedup_flagged": false,
  "contradiction_flagged": false,
  "duplicate_warning": null
}
```

`duplicate_warning` is `null` when no duplicate detected, or:
```json
{
  "existing_id": "6ba7b810-9dad-11d1-80b4-00c04fd43050",
  "existing_title": "Use parameterized queries for FTS5",
  "similarity": 0.96,
  "title_match": "exact",
  "suggestion": "This rule may duplicate existing rule 6ba7b810-...3050. If updating it, consider superseding 6ba7b810-...3050 instead. If distinct, ignore this warning."
}
```

**Side effects:**
- Writes markdown file to `.cogz/rules/<slug>.md`
- Syncs to DB (entity + embedding + FTS)
- Runs dedup + title match + contradiction checks
- Records domain event

---

### 4. `query_rules`

Query rules by status or references.

```json
{
  "name": "query_rules",
  "description": "Query rules with optional filters. Returns matching rules sorted by confidence then recency.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "status": {
        "type": "string",
        "enum": ["active", "stale", "superseded", "rejected", "pruned"],
        "default": "active"
      },
      "references": {
        "type": "string",
        "description": "Filter to rules referencing this entity UUID."
      },
      "limit": {
        "type": "integer",
        "default": 20
      }
    }
  }
}
```

**Returns:**
```json
{
  "rules": [
    {
      "id": "6ba7b810-9dad-11d1-80b4-00c04fd43055",
      "title": "Use parameterized queries for FTS5",
      "content": "Always use parameterized queries when constructing FTS5 search...",
      "status": "active",
      "confidence": 1.0,
      "references": ["a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023"],
      "created_at": "2026-08-27T15:00:00Z",
      "updated_at": "2026-08-27T15:00:00Z",
      "file_path": ".cogz/rules/use-parameterized-queries-for-fts5.md"
    }
  ],
  "count": 1
}
```

---

### 5. `create_knowledge`

Create a knowledge entry — structured documentation about the
codebase.

```json
{
  "name": "create_knowledge",
  "description": "Create a knowledge entry — structured documentation about the codebase. Knowledge is human-readable, git-tracked, and meant to be read by both humans and agents.",
  "inputSchema": {
    "type": "object",
    "required": ["title", "content", "category"],
    "properties": {
      "title": {
        "type": "string",
        "description": "Title of the knowledge entry."
      },
      "content": {
        "type": "string",
        "description": "Markdown content of the knowledge entry."
      },
      "category": {
        "type": "string",
        "description": "Category for organization (e.g., 'architecture', 'decisions', 'patterns'). Determines subdirectory under .cogz/knowledge/."
      },
      "tags": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Optional tags for additional organization."
      },
      "references": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Entity UUIDs of code entities this knowledge relates to."
      }
    }
  }
}
```

**Returns:**
```json
{
  "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "file_path": ".cogz/knowledge/architecture/search-design.md",
  "status": "active",
  "dedup_flagged": false,
  "duplicate_warning": null
}
```

`duplicate_warning` is `null` when no duplicate detected, or:
```json
{
  "existing_id": "f47ac10b-58cc-4372-a567-0e02b2c3d458",
  "existing_title": "Search Architecture",
  "similarity": 0.96,
  "title_match": "exact",
  "suggestion": "This knowledge entry may duplicate existing entry f47ac10b-...d458. Use update_knowledge(id=\"f47ac10b-...d458\") to update the existing entry, or ignore if this is a distinct entry."
}
```

**Side effects:**
- Writes markdown file to `.cogz/knowledge/<category>/<slug>.md`
- Syncs to DB (entity + embedding + FTS)
- Runs dedup + title match checks (sets `dedup_flagged` and
  `duplicate_warning` if similar entry exists)
- Knowledge duplicates are flagged, not auto-resolved

---

### 6. `update_knowledge`

Update an existing knowledge entry. This is the only content-edit
tool in the system — knowledge is documentation, and in-place edits
are the correct workflow. Observations and rules cannot be edited
in-place (see entity-spec.md update policy).

```json
{
  "name": "update_knowledge",
  "description": "Update an existing knowledge entry's content. Knowledge is the only entity type that allows in-place content edits — observations and rules are append-only. The file is overwritten and the DB is re-synced.",
  "inputSchema": {
    "type": "object",
    "required": ["id", "content"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Entity UUID of the knowledge entry to update."
      },
      "content": {
        "type": "string",
        "description": "New markdown content for the knowledge entry."
      },
      "title": {
        "type": "string",
        "description": "New title. If omitted, title is unchanged."
      },
      "category": {
        "type": "string",
        "description": "New category. If omitted, category is unchanged. Changing category moves the file to a new subdirectory."
      },
      "tags": {
        "type": "array",
        "items": { "type": "string" },
        "description": "New tags. If omitted, tags are unchanged."
      },
      "references": {
        "type": "array",
        "items": { "type": "string" },
        "description": "New references. If omitted, references are unchanged."
      }
    }
  }
}
```

**Returns:**
```json
{
  "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "file_path": ".cogz/knowledge/architecture/search-design.md",
  "status": "active",
  "updated_fields": ["content", "tags"]
}
```

**Side effects:**
- Overwrites the existing markdown file with new content
- If category changed: moves file to new subdirectory
- Syncs to DB (re-embeds, updates FTS, updates entity)
- Records `knowledge_updated` domain event

**Errors:**
- `entity_not_found` — UUID does not exist or is not a knowledge entity
- `file_write_failed` — could not write the file

---

### 7. `query_knowledge`

Query knowledge entries.

```json
{
  "name": "query_knowledge",
  "description": "Query knowledge entries with optional filters.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "category": {
        "type": "string",
        "description": "Filter by category."
      },
      "tags": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Filter by tags (any match)."
      },
      "status": {
        "type": "string",
        "default": "active"
      },
      "limit": {
        "type": "integer",
        "default": 20
      }
    }
  }
}
```

**Returns:**
```json
{
  "knowledge": [
    {
      "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
      "title": "Search Architecture",
      "content": "# Search Architecture\n\nThe search system uses...",
      "category": "architecture",
      "tags": ["search", "fts5", "vector"],
      "status": "active",
      "references": ["a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023"],
      "created_at": "2026-08-27T09:00:00Z",
      "updated_at": "2026-08-27T09:00:00Z",
      "file_path": ".cogz/knowledge/architecture/search-design.md"
    }
  ],
  "count": 1
}
```

---

### 8. `search`

Hybrid search across all entity types.

```json
{
  "name": "search",
  "description": "Search across all entities (observations, rules, knowledge, code) using hybrid FTS5 + vector search with RRF fusion. Results include graph expansion — related entities found by following edges.",
  "inputSchema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query."
      },
      "entity_type": {
        "type": "string",
        "enum": ["observation", "rule", "knowledge", "function", "class", "file", "module"],
        "description": "Filter to a specific entity type. Default: all types."
      },
      "status": {
        "type": "string",
        "enum": ["active", "stale", "superseded", "rejected", "pruned", "all"],
        "default": "active",
        "description": "Filter by status. Default: 'active' (excludes stale/superseded/rejected/pruned). Use 'all' to include everything."
      },
      "limit": {
        "type": "integer",
        "default": 20,
        "description": "Max results before graph expansion."
      },
      "expand": {
        "type": "boolean",
        "default": true,
        "description": "Whether to perform graph expansion from search results."
      }
    }
  }
}
```

**Returns:**
```json
{
  "results": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440042",
      "type": "observation",
      "title": "FTS5 ranking bug in _build_fts_search_sql",
      "content": "The RRF fusion produces incorrect rankings...",
      "relevance": 0.92,
      "graph_path": ["550e8400-e29b-41d4-a716-446655440042", "a1b2c3d4-e5f6-4789-abcd-000000000017", "a1b2c3d4-e5f6-4789-abcd-000000000023"],
      "graph_path_description": "observation → references → function _build_fts_search_sql → calls → function _fts_search_all"
    }
  ],
  "count": 1,
  "search_mode": "hybrid"
}
```

`search_mode` is one of: `hybrid` (FTS + vec), `fts_only` (when
embedding model unavailable).

---

### 9. `get_context`

Assemble a context pack for the agent. This is the primary output of
CogZ.

```json
{
  "name": "get_context",
  "description": "Assemble a context pack — a coherent, scoped, ranked collection of information for the current task. This is the primary output of CogZ. Includes provenance (graph paths showing why each piece was included).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "The task or prompt to assemble context for. Required for 'task' and 'escalation' modes."
      },
      "mode": {
        "type": "string",
        "enum": ["cold_start", "task", "escalation"],
        "default": "task",
        "description": "cold_start: session-start pack (repo identity, recent rules, recent observations). task: query-scoped ranked retrieval with graph expansion. escalation: wider retrieval when task pack was insufficient."
      },
      "include_stale": {
        "type": "boolean",
        "default": false,
        "description": "If true, include stale entities in the context pack. Default: false (only active entities). Stale entities are included with a warning in their section metadata."
      },
      "max_tokens": {
        "type": "integer",
        "description": "Token budget for the context pack. Default: from config."
      }
    }
  }
}
```

**Returns:**
```json
{
  "query": "FTS5 ranking bug",
  "mode": "task",
  "sections": [
    {
      "source": "observation",
      "entity_id": "550e8400-e29b-41d4-a716-446655440042",
      "title": "FTS5 ranking bug in _build_fts_search_sql",
      "content": "The RRF fusion produces incorrect rankings when k=60...",
      "relevance": 0.92,
      "graph_path": ["550e8400-e29b-41d4-a716-446655440042", "a1b2c3d4-e5f6-4789-abcd-000000000017"]
    },
    {
      "source": "function",
      "entity_id": "a1b2c3d4-e5f6-4789-abcd-000000000017",
      "title": "_build_fts_search_sql",
      "content": "fn _build_fts_search_sql(...) -> String { ... }",
      "relevance": 0.85,
      "graph_path": ["550e8400-e29b-41d4-a716-446655440042", "a1b2c3d4-e5f6-4789-abcd-000000000017"]
    },
    {
      "source": "rule",
      "entity_id": "6ba7b810-9dad-11d1-80b4-00c04fd43055",
      "title": "Use parameterized queries for FTS5",
      "content": "Always use parameterized queries when constructing FTS5...",
      "relevance": 0.78,
      "graph_path": ["6ba7b810-9dad-11d1-80b4-00c04fd43055", "a1b2c3d4-e5f6-4789-abcd-000000000017"]
    }
  ],
  "metadata": {
    "size_tokens": 847,
    "selected_sources": ["observation", "function", "rule"],
    "dropped_sources": ["knowledge:search-design (over token budget)"],
    "search_mode": "hybrid"
  }
}
```

---

### 10. `consolidate`

Trigger consolidation manually. Normally consolidation happens on
every insert, but this runs the background phases (promotion, merge)
explicitly.

```json
{
  "name": "consolidate",
  "description": "Trigger background consolidation: promote supported observations to rules, merge confirmed duplicates. Dedup and contradiction detection happen automatically on every insert; this tool runs the deferred phases.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "dry_run": {
        "type": "boolean",
        "default": false,
        "description": "If true, report what would be consolidated without making changes."
      }
    }
  }
}
```

**Returns:**
```json
{
  "promoted": [
    {"observation_id": "550e8400-e29b-41d4-a716-446655440042", "new_rule_id": "6ba7b810-9dad-11d1-80b4-00c04fd43055", "reason": "3 supporting observations, code structure validated"}
  ],
  "merged": [
    {"survivor_id": "6ba7b810-9dad-11d1-80b4-00c04fd43055", "superseded_id": "550e8400-e29b-41d4-a716-446655440048", "reason": "0.95 embedding similarity, same references"}
  ],
  "promoted_count": 1,
  "merged_count": 1,
  "dry_run": false
}
```

---

### 11. `get_status`

Get system status — DB stats, model availability, entity counts.

```json
{
  "name": "get_status",
  "description": "Get CogZ system status: database stats, model availability, entity counts by type, stale entity count.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}
```

**Returns:**
```json
{
  "version": "0.1.0",
  "schema_version": 3,
  "db_path": ".cogz/cogz.db",
  "db_size_bytes": 1048576,
  "entities": {
    "observation": 45,
    "rule": 12,
    "knowledge": 8,
    "function": 234,
    "class": 56,
    "file": 89,
    "module": 12
  },
  "total_entities": 456,
  "stale_count": 3,
  "edges": 1245,
  "events": 789,
  "models": {
    "embedding_code": {"available": true, "name": "BAAI/bge-small-en-v1.5"},
    "embedding_knowledge": {"available": true, "name": "BAAI/bge-small-en-v1.5"},
    "nli": {"available": false, "name": null}
  },
  "last_index": "2026-08-27T14:00:00Z"
}
```

---

### 12. `capture_event`

Capture a lifecycle event from hook scripts. Called by
`cogz capture-event` CLI, which is called by hook scripts.

```json
{
  "name": "capture_event",
  "description": "Capture a lifecycle event. Called by hook scripts (session_start, prompt_submit, pre_tool_use, post_tool_use, file_save, session_end, stop). For session_start and prompt_submit, returns a context pack for injection.",
  "inputSchema": {
    "type": "object",
    "required": ["event_type"],
    "properties": {
      "event_type": {
        "type": "string",
        "enum": ["session_start", "prompt_submit", "pre_tool_use", "post_tool_use", "file_save", "session_end", "stop"],
        "description": "The lifecycle event type."
      },
      "prompt": {
        "type": "string",
        "description": "The prompt text (for prompt_submit)."
      },
      "tool_name": {
        "type": "string",
        "description": "The tool name (for pre/post_tool_use)."
      },
      "tool_result": {
        "type": "string",
        "description": "The tool result summary (for post_tool_use)."
      },
      "file_path": {
        "type": "string",
        "description": "Saved file path, relative to repo root (for file_save)."
      }
    }
  }
}
```

**Returns (session_start):**
```json
{
  "event_type": "session_start",
  "event_id": 100,
  "context_pack": {
    "mode": "cold_start",
    "sections": [...],
    "metadata": {...}
  }
}
```

**Returns (prompt_submit):**
```json
{
  "event_type": "prompt_submit",
  "event_id": 101,
  "context_pack": {
    "mode": "task",
    "sections": [...],
    "metadata": {...}
  }
}
```

**Returns (pre/post_tool_use):**
```json
{
  "event_type": "pre_tool_use",
  "event_id": 102,
  "observation_id": null,
  "context_pack": null
}
```

For `pre_tool_use` and `post_tool_use`, if the event is deemed
meaningful (e.g., a file was modified, a test was run), an
observation is recorded and `observation_id` is set. Otherwise
`observation_id` is null.

---

### 13. `list_entities`

List all entities of a given type. No filtering, no ranking —
enumeration. Results are capped at 1000 entries; for larger sets,
use `query_*` tools with pagination via `limit`.

```json
{
  "name": "list_entities",
  "description": "List all entities of a given type. Returns IDs and titles only — use query tools for full content. Useful for browsing what exists.",
  "inputSchema": {
    "type": "object",
    "required": ["entity_type"],
    "properties": {
      "entity_type": {
        "type": "string",
        "enum": ["observation", "rule", "knowledge", "function", "class", "file", "module"],
        "description": "The entity type to list."
      },
      "status": {
        "type": "string",
        "default": "active"
      }
    }
  }
}
```

**Returns:**
```json
{
  "entity_type": "rule",
  "entities": [
    {"id": "6ba7b810-9dad-11d1-80b4-00c04fd43055", "title": "Use parameterized queries for FTS5"},
    {"id": "6ba7b810-9dad-11d1-80b4-00c04fd43056", "title": "Always validate input before FTS5 search"},
    {"id": "6ba7b810-9dad-11d1-80b4-00c04fd43057", "title": "Use RRF k=60 for search fusion"}
  ],
  "count": 3
}
```

---

## Error Responses

All tools return MCP error responses on failure:

```json
{
  "error": {
    "code": "file_write_failed",
    "message": "Failed to write observation file: Permission denied (.cogz/observations/2026-08/)"
  }
}
```

Error codes:
- `file_write_failed` — could not write the entity file
- `entity_not_found` — referenced entity UUID does not exist
- `invalid_parameter` — parameter validation failed
- `db_error` — SQLite operation failed
- `model_unavailable` — requested model is not loaded (non-fatal, tool may still succeed with degraded behavior)

---

## Tool Count: 13

| # | Tool | Phase |
|---|---|---|
| 1 | `record_observation` | 7 |
| 2 | `query_observations` | 7 |
| 3 | `create_rule` | 7 |
| 4 | `query_rules` | 7 |
| 5 | `create_knowledge` | 7 |
| 6 | `update_knowledge` | 7 |
| 7 | `query_knowledge` | 7 |
| 8 | `search` | 7 |
| 9 | `get_context` | 7 |
| 10 | `consolidate` | 9 |
| 11 | `get_status` | 2 |
| 12 | `capture_event` | 11 |
| 13 | `list_entities` | 2 |

All 13 are defined. No speculative tools. Each maps to a specific
implementation phase. `update_knowledge` is the only content-edit
tool — observations and rules are append-only by design.
