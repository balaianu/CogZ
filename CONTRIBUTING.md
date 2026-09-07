# Contributing to CogZ

CogZ is a local-first, code-aware engineering cognition runtime written in Rust. This guide covers building, testing, and proposing changes.

## Prerequisites

- **Rust 1.88+** (edition 2024)
- **Git**
- **C compiler** — `rusqlite` bundles SQLite and needs a C toolchain (`gcc`/`clang` on Linux, Xcode CLT on macOS, MSVC on Windows)
- **~2 GB free RAM** for a release build (LTO + fat codegen-units)

No ONNX models or GPU are required to build or run tests. Tests use mock models and run fully offline.

## Build

```bash
cargo build --release    # optimized binary at target/release/cogz
cargo build              # faster compile, slower runtime
```

## Test

```bash
cargo test               # all tests (library + integration)
cargo test --lib         # library tests only
cargo test --test <name> # a specific integration test
```

Tests run offline. No network access or model downloads are needed.

## Lint and format

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Both must pass before submitting a PR. CI enforces these.

## Architecture invariants

Before making changes, read `docs/design/architecture.md` and `docs/dev/conventions.md`. The most important invariants:

1. **Files are canonical, DB is derived.** Every write goes to a Markdown file first, then syncs to SQLite. Never write directly to the DB outside `src/storage/`.
2. **Entity IDs are UUIDs.** File-backed entities use UUID v4; code entities use deterministic UUID v5. No integer IDs.
3. **Single SQLite connection behind a mutex.** No connection pool. DB calls from async MCP handlers go through `spawn_blocking`.
4. **Graceful degradation.** The system must function without ONNX models. FTS-only mode is always available.
5. **400 lines max per file.** Extract self-contained functions to a new module when a file approaches this limit.

## Proposing changes

1. **Open an issue** describing the problem or feature.
2. **Fork and branch** from `master`.
3. **Write tests** alongside the implementation. Every non-trivial change needs tests — see `docs/dev/testing.md` for guidance.
4. **Verify:** `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` all pass.
5. **Keep commits focused.** One logical change per commit. Write commit messages that explain *why*, not *what*.
6. **Open a PR** referencing the issue.

## Project structure

```
src/
  main.rs              CLI entry point (clap)
  cli.rs               CLI command dispatch
  commands/            CLI command implementations
  config/              Typed config (TOML)
  storage/             SQLite layer (entities, edges, FTS5, vec0, events)
  files/               File I/O, frontmatter parser, file→DB sync
  embed/               ONNX model loading, inference, caching, download
  search/              Hybrid FTS + vector search, RRF, graph expansion
  context/             Context pack assembly, token budgeting
  consolidate/         Dedup, contradiction, promotion, merge
  index/               Code indexing (tree-sitter), git diff, stale flagging
  mcp/                 MCP server, tool handlers, params
  hooks/               Lifecycle event handlers
  doctor/              Health checks
  init.rs              cogz init
  update.rs            Self-update from GitHub releases
tests/                 Integration tests
docs/                  Documentation
```

## License

MIT. By contributing, you agree your contributions are licensed under the same terms.
