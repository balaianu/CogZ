# CogZ

Local-first, code-aware engineering cognition runtime for AI coding agents.

CogZ gives a coding agent persistent memory, contextual retrieval, and
continuous cognition about a software repository — all running locally
on your machine, no cloud services required.

## What it does

- **Memory** — stores observations, rules, and knowledge about a
  codebase as Markdown files, structured by taxonomy and linked to the
  code itself. Memory persists across sessions.
- **Context** — assembles scoped context packs with graph provenance:
  relevant observations, rules, knowledge, and code structures, ranked
  and traceable.
- **Cognition** — continuously consolidates memory: deduplicates
  entries, detects contradictions, promotes supported observations to
  rules, and flags stale knowledge when code changes.

## Quick start

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/balaianu/CogZ/main/install.sh | bash

# Initialize in a repo
cd ~/your-project
cogz init

# Index (downloads models on first run, or use --no-download for FTS-only)
cogz index

# Search
cogz search "authentication flow"

# Assemble context for an agent
cogz context --mode task "implement rate limiting"
```

## MCP integration

Configure CogZ as an MCP server in your agent's config:

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"],
      "env": {
        "COGZ_REPO": "/home/user/projects/my-project"
      }
    }
  }
}
```

The MCP server exposes 13 tools: `record_observation`,
`query_observations`, `create_rule`, `query_rules`, `create_knowledge`,
`update_knowledge`, `query_knowledge`, `search`, `get_context`,
`get_status`, `consolidate`, `capture_event`, `list_entities`.

## Hook integration

Hooks capture lifecycle events and inject context:

```json
{
  "hooks": {
    "session_start": "cogz capture-event session_start",
    "prompt_submit": "cogz capture-event prompt_submit --prompt-file $PROMPT_FILE"
  }
}
```

## CLI commands

| Command | Description |
|---|---|
| `cogz init` | Initialize `.cogz/` in a repository |
| `cogz index` | Sync files to DB + index source code |
| `cogz reindex` | Incremental reindex (changed files only) |
| `cogz search <query>` | Hybrid FTS + vector search |
| `cogz context --mode <mode> [query]` | Assemble context pack |
| `cogz status` | DB stats, entity counts, model status |
| `cogz consolidate` | Run dedup, promotion, merge |
| `cogz capture-event <type>` | Capture lifecycle event from hooks |
| `cogz models <sub>` | Model management (download, list, clean) |
| `cogz doctor` | Health check + policy violation detection |
| `cogz doctor --prune-observations` | Report/confirm observation pruning |
| `cogz update` | Self-update from GitHub releases |
| `cogz reset [--purge]` | Drop DB (optionally purge observations) |
| `cogz mcp-stdio` | Run MCP server over stdio |

## Architecture

- **Single Rust binary** — no runtime dependencies except optional
  ONNX models for vector search.
- **Files are canonical** — all entities are Markdown files. The SQLite
  DB is a derived index, disposable and rebuildable.
- **Code-aware** — tree-sitter indexes Rust and Python source code as
  first-class graph entities.
- **Graceful degradation** — works without ML models in FTS-only mode.
- **Local-first** — no cloud, no telemetry, no accounts. The only
  network access is optional model downloads.

## Documentation

See `docs/` for detailed design documents:

- `goal.md` — what CogZ is and isn't
- `first-principles.md` — axiomatic design principles
- `architecture.md` — schema, modules, concurrency, search, context
- `entity-spec.md` — file format, frontmatter, state machine
- `mcp-contract.md` — MCP tool signatures and return shapes
- `implementation-plan.md` — 12-phase build plan
- `testing-strategy.md` — test categories and coverage
- `dependencies.md` — pinned crate versions
- `packaging.md` — distribution, install, update, uninstall

## License

MIT
