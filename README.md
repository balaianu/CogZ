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

**Linux / macOS:**
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

**Windows (PowerShell):**
```powershell
# Install
irm https://raw.githubusercontent.com/balaianu/CogZ/main/install.ps1 | iex

# Initialize in a repo
cd your-project
cogz init
cogz index
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

## Requirements

### Minimum (FTS-only mode)

| Resource | Requirement |
|---|---|
| RAM | 256 MB free |
| Disk | 50 MB (binary + DB, no models) |
| CPU | any x86_64 or ARM64 |

Works without ONNX Runtime or model downloads. All hooks, FTS search,
context packs, consolidation, doctor, and prune are functional. Vector
search, embedding-based dedup, and contradiction detection are not
available.

### Recommended (hybrid search mode)

| Resource | Requirement |
|---|---|
| RAM | 2 GB free |
| Disk | 550 MB (binary + ONNX Runtime + 3 models + DB) |
| CPU | any x86_64 or ARM64, 4+ cores speeds up batch embedding |

Full functionality including vector search, semantic dedup, and NLI
contradiction detection. Models auto-download on first use and
auto-unload after 5 min idle (RAM drops back to ~11 MB). See
`docs/evaluations/2026-09-04-resource-profile.md` for the full
resource consumption profile.

## Architecture

- **Single Rust binary** — no runtime dependencies except optional
  ONNX models for vector search.
- **Files are canonical** — all entities are Markdown files. The SQLite
  DB is a derived index, disposable and rebuildable.
- **Code-aware** — tree-sitter indexes source code as first-class
  graph entities. Supported languages: Rust, Python, Go, JavaScript,
  TypeScript, Bash.
- **Graceful degradation** — works without ML models in FTS-only mode.
- **Local-first** — no cloud, no telemetry, no accounts. The only
  network access is optional model downloads.

## Compatibility

| Platform | Support | Embeddings | FTS-only | Install |
|---|---|---|---|---|
| Linux x86_64 | Full | Auto-download | Yes | `install.sh` |
| Linux aarch64 | Full | Auto-download | Yes | `install.sh` |
| macOS arm64 (Apple Silicon) | Full | Auto-download | Yes | `install.sh` |
| macOS x86_64 (Intel) | Not supported | — | — | — |
| Windows x86_64 | Full | Auto-download | Yes | `install.ps1` |

**macOS Intel** is not supported because Microsoft dropped ONNX
Runtime macOS Intel binaries after v1.22. Intel Mac users can run
the arm64 binary under Rosetta 2 (with a compatible ORT build) or
use `cargo install cogz` for FTS-only mode.

**Windows 10+** is required (bsdtar is bundled since build 17063,
needed for ONNX Runtime auto-extraction).

Cross-platform team collaboration is supported: code entity UUIDs
use forward-slash path normalization so the same source file
produces the same entity ID on all platforms.

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
