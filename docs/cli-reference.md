# CLI Reference

Every command accepts `--repo <path>` to specify the repository root (defaults to `.`). Commands that need a database require `cogz init` and `cogz index` to have been run first.

## `cogz init`

Initialize `.cogz/` in a repository.

```
cogz init [--repo <path>] [--local-only]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--local-only` — gitignore all of `.cogz/` (nothing committed). Default is team-sharing mode: knowledge, rules, and observations are committed; only the DB is gitignored.

**Output:** Creates `.cogz/` with `config.toml`, `.gitignore`, and subdirectories (`knowledge/`, `rules/`, `observations/`). Refuses to run if `.cogz/` already exists — use `cogz reset` first.

## `cogz index`

Sync entity files to the database and index source code.

```
cogz index [--repo <path>] [--no-download]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--no-download` — skip model download. Operates in FTS-only mode. Files sync without embeddings; code entities are indexed without vector embeddings.

**What it does:**
1. Scans `.cogz/knowledge/`, `.cogz/rules/`, `.cogz/observations/` and syncs to the DB (new files created, changed files updated, deleted files marked stale).
2. Scans source files (respecting `.gitignore` + `[index].allow`/`deny`) and indexes them with tree-sitter.
3. If `auto_download = true` and `--no-download` is not set, downloads models on first run.
4. Embeds all new and updated entities using the configured models.

## `cogz reindex`

Incremental reindex — only re-processes files that changed since the last index (detected via git diff).

```
cogz reindex [--repo <path>]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)

Falls back to a full index if git is unavailable or no baseline commit is stored. After syncing changed code entities, flags observations and rules referencing changed code as stale.

## `cogz search`

Hybrid FTS5 + vector search with graph expansion.

```
cogz search <query> [--repo <path>] [--entity-type <type>] [--status <status>] [--limit <n>] [--no-expand] [--code]
```

**Arguments:**
- `<query>` — search query (required)

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--entity-type <type>` — filter by type: `observation`, `rule`, `knowledge`, `function`, `class`, `file`, `module`
- `--status <status>` — filter by status (default: `active`). Use `all` for all statuses.
- `--limit <n>` — max results before graph expansion (default: from config)
- `--no-expand` — disable graph expansion
- `--code` — use the code model (CodeRankEmbed) for query embedding. Applies the CodeRankEmbed query prefix for code-focused search.

## `cogz context`

Assemble a context pack for agent consumption.

```
cogz context [--mode <mode>] [query] [--repo <path>] [--include-stale] [--max-tokens <n>]
```

**Arguments:**
- `[query]` — search query (required for `task` and `escalation` modes)

**Flags:**
- `--mode <mode>` — context mode: `cold_start`, `task`, `escalation` (default: `task`)
- `--repo <path>` — repository root (default: `.`)
- `--include-stale` — include stale entities in the context pack
- `--max-tokens <n>` — override the token budget from config

**Output:** JSON context pack with sections (entity title + content), metadata (token count, search mode, dropped sources), and graph provenance.

## `cogz status`

Show system status: DB stats, entity counts, model availability.

```
cogz status [--repo <path>]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)

## `cogz consolidate`

Run background consolidation: promote supported observations to rules, merge confirmed duplicates.

```
cogz consolidate [--repo <path>] [--dry-run]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--dry-run` — report what would be consolidated without making changes

Dedup and contradiction detection happen automatically on every insert. This command runs the deferred phases (promotion and merge). Merge candidates are confirmed by NLI bidirectional entailment before any merge occurs.

## `cogz capture-event`

Capture a lifecycle event. Called by agent hook systems or manually.

```
cogz capture-event <event_type> [--repo <path>] [--prompt <text>] [--prompt-file <path>] [--tool-name <name>] [--tool-result <summary>] [--file-path <path>] [--hook-json] [--fts-only]
```

**Arguments:**
- `<event_type>` — one of: `session_start`, `prompt_submit`, `pre_tool_use`, `post_tool_use`, `file_save`, `session_end`, `stop`

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--prompt <text>` — prompt text (for `prompt_submit`)
- `--prompt-file <path>` — read prompt from a file (for `prompt_submit`)
- `--tool-name <name>` — tool name (for `pre_tool_use`, `post_tool_use`)
- `--tool-result <summary>` — tool result summary (for `post_tool_use`)
- `--file-path <path>` — saved file path, relative to repo root (for `file_save`)
- `--hook-json` — wrap output as JSON for agent hook systems (`{"hookSpecificOutput": {...}}`). Silent skip if no `.cogz/` found.
- `--fts-only` — skip model loading. Use FTS-only search for context assembly. Much faster (~2s vs ~40s) but lower quality ranking. Recommended for hook calls.

**Behavior by event type:**
- `session_start` — records event, assembles cold_start context pack, prints to stdout
- `prompt_submit` — records event, assembles task context pack using the prompt, prints to stdout
- `pre_tool_use` — records event only (audit trail)
- `post_tool_use` — records event only (audit trail)
- `file_save` — records event, triggers incremental code reindex and stale-knowledge flagging if the file is a source file
- `session_end` — records event, runs consolidation (promotion + merge)
- `stop` — records event, no side effects

See [Hooks](integration/hooks.md) for hook configuration examples.

## `cogz models`

Model management.

### `cogz models download`

```
cogz models download [--repo <path>] [--code] [--knowledge] [--nli]
```

Download configured models from HuggingFace. Without flags, downloads all three. With `--code`, `--knowledge`, or `--nli`, downloads only the specified model.

### `cogz models list`

```
cogz models list [--repo <path>]
```

Show configured models and download status.

### `cogz models clean`

```
cogz models clean
```

Remove broken cache files (`.incomplete` files >1h old, empty `refs/main`). Safe to run anytime.

## `cogz doctor`

Health check — DB integrity, model availability, policy violations.

```
cogz doctor [--repo <path>] [--prune-observations] [--confirm]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--prune-observations` — report prunable observations (dry-run)
- `--confirm` — confirm pruning (deletes content, creates tombstones). Only prunes `rejected` and `superseded` observations older than `retention.observation_prune_after_days`.

**Checks performed:**
- DB integrity (`PRAGMA integrity_check`)
- Schema version match
- Model availability on disk
- File-sync consistency (missing files, missing entities, stale-not-flagged)
- Observation content edits (append-only violation)
- Orphaned supersedes (superseded entity without `superseded_by`)
- Near-duplicate knowledge (embedding similarity > 0.80)
- Vector dimension mismatch (config change after DB creation)
- Corrupt JSON in entity properties or event payloads
- Corrupt embedding blobs

## `cogz update`

Self-update from GitHub releases.

```
cogz update [--check]
```

**Flags:**
- `--check` — check if an update is available without downloading or installing. Prints `CogZ is up to date (v...)` or `Update available: v... → v...`.

Without `--check`, downloads the latest release binary, verifies the SHA256 checksum, and atomically replaces the current binary.

## `cogz reset`

Drop the database. Rebuildable from files via `cogz index`.

```
cogz reset [--repo <path>] [--purge]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--purge` — also remove observations and the generated `.gitignore`. Keeps knowledge, rules, and config.

## `cogz mcp-stdio`

Run the MCP server over stdio for AI agent integration.

```
cogz mcp-stdio
```

No flags. The server starts empty — every tool call must specify `repo` explicitly. See [MCP Tools](integration/mcp-tools.md) for the tool reference.

## `cogz --version`

Print the version and exit.

## `cogz --help`

Print help and exit.
