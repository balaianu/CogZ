# CogZ

[![CI](https://github.com/balaianu/CogZ/actions/workflows/ci.yml/badge.svg)](https://github.com/balaianu/CogZ/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![Version](https://img.shields.io/badge/version-0.1.0-green.svg)](https://github.com/balaianu/CogZ/releases)
[![Buy Me A Coffee](https://img.shields.io/badge/☕-Buy%20Me%20A%20Coffee-yellow)](https://buymeacoffee.com/balaianu)

Local-first, code-aware engineering cognition runtime for AI coding agents.

CogZ gives a coding agent persistent memory, contextual retrieval, and continuous cognition about a software repository — all running locally on your machine, no cloud services required.

Works with Devin, Claude Code, Cursor, Codex, Windsurf, and any MCP-compatible agent.

## What it does

- **Memory** — stores observations, rules, and knowledge about a codebase as Markdown files, structured by taxonomy and linked to the code itself. Memory persists across sessions.
- **Context** — assembles scoped context packs with graph provenance: relevant observations, rules, knowledge, and code structures, ranked and traceable.
- **Cognition** — continuously consolidates memory: deduplicates entries, detects contradictions, promotes supported observations to rules, and flags stale knowledge when code changes.

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

See [Getting Started](docs/getting-started.md) for the mental model and a complete walkthrough.

## MCP integration

CogZ runs as a stateless MCP server over stdio, aligned with the 2026-07-28 MCP spec (SEP-2577). Every tool call specifies which repo it targets via a required `repo` parameter — no Roots, no session state, no fallbacks.

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

The server exposes 13 tools: `record_observation`, `query_observations`, `create_rule`, `query_rules`, `create_knowledge`, `update_knowledge`, `query_knowledge`, `search`, `get_context`, `get_status`, `list_entities`, `consolidate`, `capture_event`.

See [MCP Tools](docs/integration/mcp-tools.md) for full parameter reference and example responses. See [Agent Setup](docs/integration/agent-setup.md) for configuration examples for Devin, Claude Code, and other agents.

## Hook integration

Hooks capture lifecycle events and inject context packs into agent sessions. CogZ's binary is the hook handler — no wrapper scripts needed.

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "cogz capture-event session_start --hook-json --fts-only",
        "timeout": 10
      }]
    }]
  }
}
```

See [Hooks](docs/integration/hooks.md) for all 7 event types and per-agent wiring guides.

## CLI commands

| Command | Description |
|---|---|
| `cogz init` | Initialize `.cogz/` in a repository |
| `cogz index [--no-download]` | Sync files to DB + index source code |
| `cogz reindex` | Incremental reindex (changed files only) |
| `cogz search <query>` | Hybrid FTS + vector search |
| `cogz context --mode <mode> [query]` | Assemble context pack |
| `cogz status` | DB stats, entity counts, model status |
| `cogz consolidate [--dry-run]` | Run promotion and merge |
| `cogz capture-event <type>` | Capture lifecycle event from hooks |
| `cogz models <download\|list\|clean>` | Model management |
| `cogz doctor [--prune-observations]` | Health check + policy violations |
| `cogz update [--check]` | Self-update from GitHub releases |
| `cogz reset [--purge]` | Drop DB (optionally purge observations) |
| `cogz mcp-stdio` | Run MCP server over stdio |

See [CLI Reference](docs/cli-reference.md) for all flags and options.

## Requirements

### Minimum (FTS-only mode)

| Resource | Requirement |
|---|---|
| RAM | 256 MB free |
| Disk | 50 MB (binary + DB, no models) |
| CPU | any x86_64 or ARM64 |

Works without ONNX Runtime or model downloads. All hooks, FTS search, context packs, consolidation, doctor, and prune are functional. Vector search, embedding-based dedup, and contradiction detection are not available.

### Recommended (hybrid search mode)

| Resource | Requirement |
|---|---|
| RAM | 2 GB free |
| Disk | 550 MB (binary + ONNX Runtime + 3 models + DB) |
| CPU | any x86_64 or ARM64, 4+ cores speeds up batch embedding |

Full functionality including vector search, semantic dedup, and NLI contradiction detection. Models auto-download on first use and auto-unload after 5 min idle (RAM drops back to ~11 MB). See [Evaluations](docs/evaluations/) for the full resource consumption profile.

## Architecture

- **Single Rust binary** — no runtime dependencies except optional ONNX models for vector search.
- **Files are canonical** — all entities are Markdown files. The SQLite DB is a derived index, disposable and rebuildable.
- **Code-aware** — tree-sitter indexes source code as first-class graph entities. Supported languages: Rust, Python, Go, JavaScript, TypeScript, TSX, Bash.
- **Graceful degradation** — works without ML models in FTS-only mode.
- **Local-first** — no cloud, no telemetry, no accounts. The only network access is optional model downloads.

See [Architecture](docs/design/architecture.md) for the full system design.

## Compatibility

| Platform | Support | Embeddings | FTS-only | Install |
|---|---|---|---|---|
| Linux x86_64 | Full | Auto-download | Yes | `install.sh` |
| Linux aarch64 | Full | Auto-download | Yes | `install.sh` |
| macOS arm64 (Apple Silicon) | Full | Auto-download | Yes | `install.sh` |
| macOS x86_64 (Intel) | Not supported | — | — | — |
| Windows x86_64 | Full | Auto-download | Yes | `install.ps1` |

**macOS Intel** is not supported because Microsoft dropped ONNX Runtime macOS Intel binaries after v1.22. Intel Mac users can run the arm64 binary under Rosetta 2 (with a compatible ORT build) or use `cargo install cogz` for FTS-only mode.

**Windows 10+** is required (bsdtar is bundled since build 17063, needed for ONNX Runtime auto-extraction).

Cross-platform team collaboration is supported: code entity UUIDs use forward-slash path normalization so the same source file produces the same entity ID on all platforms.

## Documentation

**User guides:**
- [Getting Started](docs/getting-started.md) — mental model and walkthrough
- [Configuration](docs/configuration.md) — full `config.toml` reference
- [CLI Reference](docs/cli-reference.md) — every command and flag

**Integration:**
- [MCP Tools](docs/integration/mcp-tools.md) — 13 tool parameters and responses
- [Hooks](docs/integration/hooks.md) — lifecycle events and output format
- [Agent Setup](docs/integration/agent-setup.md) — Devin, Claude Code, generic MCP

**Design:**
- [Architecture](docs/design/architecture.md) — system overview and module map
- [Entity Model](docs/design/entity-model.md) — entity types, frontmatter, state machine
- [Search](docs/design/search.md) — hybrid FTS + vector, RRF, graph expansion
- [Consolidation](docs/design/consolidation.md) — dedup, contradiction, promotion, merge
- [Degradation](docs/design/degradation.md) — FTS-only mode and fallback behavior

**Contributing:**
- [Building](docs/dev/building.md) — build, release, cross-compile
- [Testing](docs/dev/testing.md) — test categories and mock models
- [Conventions](docs/dev/conventions.md) — code patterns and invariants
- [Dependencies](docs/dev/dependencies.md) — pinned versions and supply-chain policy
- [Schema](docs/dev/schema.md) — DB schema and migrations

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build, test, and PR guidelines.

## License

MIT — see [LICENSE](LICENSE).

## Support

If you find this tool useful, consider buying me a coffee:

[![Buy Me A Coffee](https://img.shields.io/badge/☕-Buy%20Me%20A%20Coffee-yellow)](https://buymeacoffee.com/balaianu)
