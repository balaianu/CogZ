# MCP Tools

CogZ exposes 13 tools via the Model Context Protocol (MCP) over stdio. The server is stateless per the 2026-07-28 MCP spec (SEP-2577) — no Roots, no sessions, no cwd inference. Every tool call must include a `repo` parameter with the absolute path to the project root containing `.cogz/`.

## Server setup

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"]
    }
  }
}
```

The server starts with no pre-loaded repos. Repos are opened and cached on first use. Models are shared across repos — if two repos use the same model, they share one ONNX session instance.

## Write tools

### `record_observation`

Record an observation about the codebase. Observations are raw, unvalidated experience — bugs found, decisions made, patterns noticed. They persist across sessions and can be promoted to rules through consolidation.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `content` | string | yes | Observation content |
| `title` | string | no | Short title. Auto-generated from content if omitted. |
| `references` | array of strings | no | UUIDs this observation references |
| `supporting_ids` | array of strings | no | UUIDs of observations this observation supports. Creates `supports` edges for promotion. |
| `source` | string | no | Who or what produced this observation. Default: `"agent"`. |

**Returns:** JSON with `id`, `title`, `status`, `duplicate_warning` (if a similar entity exists), `contradiction_flagged` (if NLI detected a contradiction).

### `create_rule`

Create a rule — a validated directive the agent should follow. Rules are git-tracked and shared.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `content` | string | yes | Rule content |
| `title` | string | no | Short title. Auto-generated if omitted. |
| `references` | array of strings | no | UUIDs this rule references |
| `confidence` | float | no | Confidence score (0.0–1.0) |

**Returns:** JSON with `id`, `title`, `status`.

### `create_knowledge`

Create a knowledge entry — structured documentation about the codebase. Knowledge is human-readable, git-tracked, and meant to be read by both humans and agents.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `title` | string | yes | Knowledge title |
| `content` | string | yes | Knowledge content |
| `category` | string | yes | Category (becomes a subdirectory under `knowledge/`) |
| `tags` | array of strings | no | Tags for filtering |
| `references` | array of strings | no | UUIDs this knowledge references |

**Returns:** JSON with `id`, `title`, `status`, `file_path`.

### `update_knowledge`

Update an existing knowledge entry's content. Knowledge is the only entity type that allows in-place content edits — observations and rules are append-only.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `id` | string | yes | UUID of the knowledge entry to update |
| `content` | string | yes | New content |
| `title` | string | no | New title (if changing) |
| `category` | string | no | New category (if changing) |
| `tags` | array of strings | no | New tags (if changing) |
| `references` | array of strings | no | New references (if changing) |

**Returns:** JSON with `id`, `title`, `status`, `file_path`.

## Query tools

### `query_observations`

Query observations with optional filters. Returns matching observations sorted by recency.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `status` | string | no | Filter by status (default: `active`) |
| `references` | string | no | Filter by reference (UUID or path) |
| `limit` | integer | no | Max results |

**Returns:** JSON array of observations with `id`, `title`, `content`, `status`, `created_at`, `references`.

### `query_rules`

Query rules with optional filters. Returns matching rules sorted by confidence then recency.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `status` | string | no | Filter by status (default: `active`) |
| `references` | string | no | Filter by reference (UUID or path) |
| `limit` | integer | no | Max results |

**Returns:** JSON array of rules with `id`, `title`, `content`, `status`, `confidence`, `created_at`, `references`.

### `query_knowledge`

Query knowledge entries with optional filters.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `category` | string | no | Filter by category |
| `tags` | array of strings | no | Filter by tags |
| `status` | string | no | Filter by status (default: `active`) |
| `limit` | integer | no | Max results |

**Returns:** JSON array of knowledge entries with `id`, `title`, `content`, `category`, `tags`, `status`, `references`.

### `list_entities`

List all entities of a given type. Returns IDs and titles only — use query tools for full content.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `entity_type` | string | yes | Entity type: `observation`, `rule`, `knowledge`, `function`, `class`, `file`, `module` |
| `status` | string | no | Filter by status (default: `active`) |

**Returns:** JSON array with `id` and `title` for each entity.

## Search tools

### `search`

Search across all entities using hybrid FTS5 + vector search with RRF fusion. Results include graph expansion — related entities found by following edges.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `query` | string | yes | Search query |
| `entity_type` | string | no | Filter by entity type |
| `status` | string | no | Filter by status (default: `active`) |
| `limit` | integer | no | Max results before expansion |
| `expand` | boolean | no | Enable graph expansion (default: `true`) |
| `code_search` | boolean | no | Use code model for query embedding (for code-focused queries) |

**Returns:** JSON with `results` array (each with `entity`, `relevance`, `graph_path`, `graph_path_description`) and `search_mode` (`hybrid`, `knowledge_hybrid`, `code_hybrid`, or `fts_only`).

### `get_context`

Assemble a context pack — a coherent, scoped, ranked collection of information for the current task. This is the primary output of CogZ.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `query` | string | no | Search query (required for `task` and `escalation` modes) |
| `mode` | string | no | Context mode: `cold_start`, `task`, `escalation` (default: `task`) |
| `include_stale` | boolean | no | Include stale entities (default: `false`) |
| `max_tokens` | integer | no | Override token budget from config |

**Returns:** JSON context pack with `query`, `mode`, `sections` (each with `source`, `entity_id`, `title`, `content`, `relevance`, `graph_path`), and `metadata` (`size_tokens`, `selected_sources`, `dropped_sources`, `search_mode`).

## System tools

### `get_status`

Get CogZ system status: database stats, model availability, entity counts by type, stale entity count.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |

**Returns:** JSON with `db_path`, `entity_counts` (by type), `stale_count`, `models` (`embedding_code`, `embedding_knowledge`, `nli` — each with `available` and `name`), `db_size_bytes`, `schema_version`.

### `consolidate`

Trigger background consolidation: promote supported observations to rules, merge confirmed duplicates. Dedup and contradiction detection happen automatically on every insert; this tool runs the deferred phases.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `dry_run` | boolean | no | Report what would be consolidated without making changes (default: `false`) |

**Returns:** JSON with `promoted` (array of promotion results) and `merged` (array of merge results with `survivor_id`, `superseded_id`, `reason`).

### `capture_event`

Capture a lifecycle event. Called by hook scripts. For `session_start` and `prompt_submit`, returns a context pack for injection. For `file_save`, triggers an incremental code reindex and stale-knowledge flagging. For `session_end`, runs consolidation.

**Parameters:**
| Name | Type | Required | Description |
|---|---|---|---|
| `repo` | string | yes | Absolute path to project root |
| `event_type` | string | yes | Event type: `session_start`, `prompt_submit`, `pre_tool_use`, `post_tool_use`, `file_save`, `session_end` |
| `prompt` | string | no | Prompt text (for `prompt_submit`) |
| `tool_name` | string | no | Tool name (for `pre_tool_use`, `post_tool_use`) |
| `tool_result` | string | no | Tool result summary (for `post_tool_use`) |
| `file_path` | string | no | Saved file path, relative to repo root (for `file_save`) |

**Returns:** JSON with `event_id`, and depending on event type: `context_pack` (for `session_start`/`prompt_submit`), `reindex_summary` (for `file_save`), `consolidation_summary` (for `session_end`).

## Error handling

All tools return MCP error responses on failure. Common errors:

- **Invalid repo path** — the path does not exist or has no `.cogz/` directory
- **Entity not found** — the specified UUID does not exist in the DB
- **Invalid entity type** — the type string is not one of the valid types
- **Storage error** — database operation failed (schema mismatch, disk full, etc.)
- **Secret detected** — content contains a high-confidence secret pattern; the write is rejected
