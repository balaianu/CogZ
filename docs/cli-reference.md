# CLI Reference

Every command accepts `--repo <path>` to specify the repository root (defaults to `.`). Commands that need a database require `cogz init` and `cogz index` to have been run first.

## `cogz init`

Initialize `.cogz/` in a repository.

```
cogz init [--repo <path>] [--local-only] [--configure <harnesses>] [--global]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--local-only` — gitignore all of `.cogz/` (nothing committed). Default is team-sharing mode: knowledge, rules, and observations are committed; only the DB is gitignored.
- `--configure <harnesses>` — also wire host-agent MCP + hook config (see `cogz configure`). Accepts comma-separated harness ids or `auto` for every detected agent.
- `--global` — with `--configure`, write user-level agent config instead of project files.

**Output:** Creates `.cogz/` with `config.toml`, `.gitignore`, and subdirectories (`knowledge/`, `rules/`, `observations/`). Also writes three **seed entries** (tagged `cogz-seed`): the first-contact ingestion protocol rule, the entry-authorship contract rule, and a knowledge entry explaining how the repo's memory works — these land in the first `session_start` cold-start pack so an agent learns the corpus mechanics through CogZ itself. Refuses to run if `.cogz/` already exists — use `cogz reset` first. Without `--configure`, prints the detected agents and the `cogz configure` hint when any are found.

## `cogz configure`

Write host-agent configuration so an agent picks up CogZ's MCP server and lifecycle hooks without hand-editing JSON. Merges into existing config files — other servers' entries and other tools' hooks are preserved, and for TOML (Codex) comments and formatting are kept — and writes a `<file>.cogz.bak` backup before overwriting an existing file. If `.cogz.bak` already exists, the backup goes to `<file>.cogz.bak.<timestamp>` instead of overwriting it. All writes are atomic (temp file + rename). Re-running is idempotent: a file that already matches is reported as already configured and left untouched.

Generated hook commands and MCP `command` fields invoke the absolute path of the running binary (resolved via `current_exe`), not a bare `cogz` — hook runtimes and MCP spawners frequently run with a minimal `PATH`. When re-running against a config written by CogZ ≤0.6.3 (or hand-wired with a bare `cogz`), configure repairs the argv[0] in place: the command's flags and the file's other entries are preserved. Paths containing whitespace are double-quoted in hook command strings; MCP `command` fields carry the raw path.

```
cogz configure <harnesses> [--repo <path>] [--global]
```

**Arguments:**
- `<harnesses>` — comma-separated ids, or `auto` to configure every detected agent: `claude-code`, `cursor`, `codex`, `gemini`, `copilot`, `devin`

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--global` — write user-level config (e.g. `~/.claude.json`, `~/.config/devin/config.json`) instead of project files

**Files written (project scope):**

| Harness | MCP | Hooks |
|---|---|---|
| `claude-code` | `.mcp.json` | `.claude/settings.json` |
| `cursor` | `.cursor/mcp.json` | `.cursor/hooks.json` |
| `codex` | `.codex/config.toml` (TOML `[mcp_servers.cogz]`) | `.codex/hooks.json` |
| `gemini` | `.gemini/settings.json` (`mcpServers` key) | same file (`hooks` key) |
| `copilot` | `.mcp.json` | `.github/hooks/cogz.json` |
| `devin` | `.devin/mcp_config.json` | `.devin/hooks.v1.json` |

Existing files that fail to parse (invalid JSON/TOML) abort the run with an error rather than being overwritten.

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
4. Embeds new, updated, and previously-unembedded active entities using the configured models. Unchanged entities are skipped by content hash — never re-embedded.

## `cogz reindex`

Incremental reindex — only re-processes files that changed since the last index (detected via git diff).

```
cogz reindex [--repo <path>]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)

Falls back to a full index if git is unavailable or no baseline commit is stored. After syncing changed code entities, flags observations and rules referencing changed code as stale.

## `cogz reindex-bg`

Background reindex — spawned by hooks (`session_start`, `prompt_submit`) to catch changes from non-hook events. Runs detached; the parent hook returns immediately.

```
cogz reindex-bg --repo <path> --db <path>
```

**Flags:**
- `--repo <path>` — repository root
- `--db <path>` — database file path

Syncs `.cogz/` entity files, runs git-diff code reindex, flags stale knowledge, then embeds synced + missing-vector entities — knowledge inline, code inline when few or via `embed-bg` when many. Not intended for manual use — spawned automatically by hooks.

## `cogz embed-bg`

Background embedding — spawned by `cogz index` or `reindex-bg` to embed code entities after indexing completes. Only spawned when the code model files exist.

```
cogz embed-bg --db <path> --ids-file <path> --code-model <name> --dimension <n> --idle-ttl <s> --min-free-mb <n>
```

**Flags:**
- `--db <path>` — database file path
- `--ids-file <path>` — temp file containing entity IDs (one per line)
- `--code-model <name>` — code embedding model name
- `--dimension <n>` — embedding dimension (must match the vec0 table)
- `--idle-ttl <s>` — model idle TTL in seconds
- `--min-free-mb <n>` — minimum free RAM (MB) required to load the model; degrades to FTS-only below it

Reads entity IDs, loads the code model, embeds in batches of 32, stores vectors, and cleans up the ID file. Not intended for manual use — spawned automatically.

## `cogz search`

Hybrid FTS5 + vector search with graph expansion.

```
cogz search <query> [--repo <path>] [--entity-type <type>] [--status <status>] [--limit <n>] [--no-expand] [--code] [--before <ts>]
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
- `--before <ts>` — only use git history committed before this unix timestamp for the co-change channel
- `--no-record` — inspection mode: read results without recording usage telemetry (access counts, delivery rows, `search_performed` events). Use for humans or tooling that look at results without consuming them.

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
- `--no-record` — inspection mode: assemble the pack without recording usage telemetry or closing open deliveries. Use for humans or tooling that inspect packs without injecting them.

**Output:** formatted text pack — header (mode, query, search mode, section/token counts, dropped sections), then numbered sections with entity type, title, relevance, drift flags, graph provenance, and content. The `get_context` MCP tool returns the same pack as JSON. Ends with a write-back nudge footer when a fresh mined candidate exists.

## `cogz status`

Show system status: DB stats, entity counts, model availability.

```
cogz status [--repo <path>]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)

## `cogz suggest`

List mined observation candidates — the review surface write-back nudges point at. Mines recent events and usage for signals (searches that missed, error→fix pairs, hot files, zero-hit deliveries, recurring reliance) and prints a drafted title/content/references for each. Nothing is written; confirming a candidate is a `create_entity` MCP call or a manual `.cogz/observations/` file.

```
cogz suggest [--repo <path>] [--days <n>] [--limit <n>]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--days <n>` — how far back to mine, in days (default: `7`, capped at `90`)
- `--limit <n>` — max candidates (default: `10`)

The CLI twin of the `suggest_observations` MCP tool — same mining pass, same `suggestions_requested` audit event, text output instead of JSON.

## `cogz consolidate`

Run background consolidation: promote supported observations to rules, merge confirmed duplicates.

```
cogz consolidate [--repo <path>] [--dry-run]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--dry-run` — report what would be consolidated without making changes

Dedup and contradiction detection happen automatically on every insert. This command runs the deferred phases (promotion and merge). Merge candidates are confirmed by NLI entailment (either direction meeting `dedup_nli_threshold`) before any merge occurs.

## `cogz capture-event`

Capture a lifecycle event. Called by agent hook systems or manually.

```
cogz capture-event <event_type> [--repo <path>] [--prompt <text>] [--prompt-file <path>] [--tool-name <name>] [--tool-result <summary>] [--file-path <path>] [--hook-json] [--hook-format <fmt>] [--fts-only] [--record-only]
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
- `--hook-format <fmt>` — stdout envelope for non-Claude-shaped agents: `cursor` emits `{"additional_context": ...}`, `copilot` emits `{"additionalContext": ...}`. Default `claude` emits `{"hookSpecificOutput": {...}}` (also correct for Codex, Gemini, Devin). Requires `--hook-json`.
- `--fts-only` — skip model loading. Use FTS-only search for context assembly. Much faster (~2s vs ~40s) but lower quality ranking. Recommended for hook calls.
- `--record-only` — run the full lifecycle (event record, reindex triggers) but suppress stdout entirely. For hook events whose output schema cannot carry context (`PostCompact`, `preCompact`, `PreCompress`); `cogz configure` wires it automatically.

**Behavior by event type:**
- `session_start` — records event, assembles cold_start context pack, prints to stdout, spawns background reindex
- `prompt_submit` — records event, assembles task context pack using the prompt, prints to stdout, spawns background reindex (debounced 60s)
- `pre_tool_use` — records event only (audit trail)
- `post_tool_use` — records event, resolves usage hits on delivered entities; may emit a write-back nudge in hook output when a fresh mined candidate exists
- `file_save` — records event, triggers single-file code reindex and stale-knowledge flagging if the file is a source file; syncs `.cogz/` file if under `.cogz/`; may emit nudges/drift notices in hook output
- `session_end` — records event, runs consolidation (promotion + merge), counts mined candidates; may emit notices in hook output
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
cogz doctor [--repo <path>] [--prune-observations] [--confirm] [--verify <mode>]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--prune-observations` — report prunable observations (dry-run)
- `--confirm` — confirm pruning (deletes content, creates tombstones). Only prunes `rejected` and `superseded` observations older than `retention.observation_prune_after_days`.
- `--verify <mode>` — run batch verification after the checks (`drifted`, `stale`, `unverified`, or `all`); same engine and output as `cogz verify --<mode>`.

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

**Usage metrics** (schema v4+):

- `Memory hit rate` — share of delivered pack entities the agent later touched (hits / delivered). Hits are detected on `post_tool_use`: a tool touching an entity's file, or a result containing the entity's id or title. Attribution is approximate — knowledge and rules are undercounted when the agent acts on their content without re-reading the file.
- `Search hit rate` — same measure for `search` tool deliveries. Printed alongside the pack rate so push vs pull efficacy can be compared.
- `Dead weight` — entities never surfaced in any delivery over the recent session window (default: last 10 sessions).
- `Write quality` — file-backed entities (observation, rule, knowledge) never delivered at all.
- `Tier <tier>` — hit rate per delivery tier (schema v5+): `baseline` (Tier-0 orientation), `full` (Tier-1 content), `pointer` (Tier-2 index entries). The measurement a future learned gate reads.
- `Type <type>` — hit rate per entity type — which kinds of memory actually get used when delivered.

Deliveries are recorded on every context pack (`session_start`, `prompt_submit`, `get_context`), every `search` call, and every targeted graph pull (`get_callers`, `get_impact`, `find_orphans`). Entities start `pending`; a tool touch marks them `hit`, and the next delivery boundary or `session_end` closes the rest as `miss`. Usage tables are disposable derived state — `cogz reset` + `cogz index` rebuilds cleanly without them.

**Review queue** (schema v6+): printed whenever stale knowledge or drifted entities exist. `[stale:<reason>]` rows are orphaned or manually-staled entities the automatic passes could not resolve; `[drift:<n>]` rows are active entities with `n` references whose `verified_against` provenance diverged. Verify accurate entries (`cogz verify`), rewrite outdated ones (`update_knowledge`).

## `cogz verify`

Re-stamp a knowledge entity's `verified_against` provenance to the current hashes of its live references. Use after confirming the content is still accurate — clears drift rows and restores full retrieval ranking. Also reactivates `code_orphaned`-stale entities whose references now resolve.

```
cogz verify <entity-id> [--allow-dead-refs] [--repo <path>]
cogz verify --drifted | --stale | --unverified | --all [--repo <path>]
```

**Flags:**
- `--allow-dead-refs` — reactivate a stale entity even though some references resolve to nothing live. The dead refs are still reported — this asserts the claim holds despite missing anchors, it does not hide them.
- `--drifted` — batch: verify every entity with drift rows.
- `--stale` — batch: verify every stale file-backed entity.
- `--unverified` — batch: verify living entities with no provenance baseline (bootstrap coverage the code-change triggers can't reach).
- `--all` — batch: verify every living file-backed entity. Selection flags are mutually exclusive with an explicit entity id.
- `--repo <path>` — repository root (default: `.`)

Verification is operator attestation: it re-stamps `verified_against` for every resolvable reference, reports `resolved N/M` plus the dead reference ids, clears the entity's drift rows, and reactivates stale entities whose anchors all resolve — or all-but-dead ones under `--allow-dead-refs`. Writes canonical frontmatter first, then re-syncs the DB. Fails on unknown IDs and entities with no backing file (code entities — they carry no `verified_against` frontmatter to re-stamp). Works on observations, rules, and knowledge. Batch mode prints a per-entity dead-ref report and a summary (`verified`, `reactivated`, `with dead refs`, `failed`).

## `cogz reject`

Reject a knowledge-layer entity — a verdict that the claim is wrong. Writes `status: rejected` into the canonical file (with an optional `rejected_reason`), then re-syncs so the status lattice validates the transition.

```
cogz reject <entity-id> [--reason <text>] [--repo <path>]
```

**Flags:**
- `--reason <text>` — why the entity is rejected; stored as `rejected_reason` in frontmatter and surfaced by dedup when a matching claim resurfaces
- `--repo <path>` — repository root (default: `.`)

`active` and `stale` observations, rules, and knowledge can be rejected — code entities carry no verdict. Rejected entities stay on record: retrieval filters them out, and they become eligible for pruning. Mirrors the `reject_entity` MCP tool.

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

Before deleting, reset checkpoints the WAL, refuses while a background worker (`reindex-bg`/`embed-bg`) holds `.bg.lock`, and takes the canonical `.cogz/.lock` (10 s bounded wait) — the DB file is never unlinked out from under an active cogz process.

```
cogz reset [--repo <path>] [--purge] [--confirm]
```

**Flags:**
- `--repo <path>` — repository root (default: `.`)
- `--purge` — also remove observations. Keeps knowledge, rules, config, and the generated `.gitignore`. Without `--confirm` this is a dry run: it reports what would be removed and changes nothing.
- `--confirm` — apply the `--purge` reset.

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
