# Dependencies

CogZ's dependency policy and pinned versions. New dependencies must follow the rules below.

## Policy

1. **No floating ranges.** Never use `*`, `latest`, or unbounded `>=`. All dependencies must have an upper bound (caret `^` for stable crates, exact `=` for pre-release crates).
2. **7-day rule.** Never use a version published less than 7 days ago. Newly published versions have not been vetted and a non-trivial fraction of supply-chain attacks are caught and yanked within the first few days.
3. **No unnecessary dependencies.** Before adding a crate, check if the functionality exists in the standard library or in a crate already in the dependency tree.
4. **Pre-release crates use exact pins.** `ort` is pinned to `=2.0.0-rc.13` because pre-release versions can have breaking changes between minor bumps.
5. **Stable crates use caret pins.** `clap = "4"` means `^4` — compatible updates are allowed.

## Current dependencies

### CLI

| Crate | Version | Purpose |
|---|---|---|
| `clap` | `4` (derive) | CLI argument parsing |

### Config / Serialization

| Crate | Version | Purpose |
|---|---|---|
| `serde` | `1` (derive) | Serialization framework |
| `serde_json` | `1` | JSON for MCP responses and event payloads |
| `toml` | `0.8` | Config file parsing |

### Embedding / ML

| Crate | Version | Purpose |
|---|---|---|
| `ort` | `=2.0.0-rc.13` | ONNX Runtime bindings (exact pin — pre-release) |
| `tokenizers` | `0.21` | HuggingFace tokenizers for model input |
| `hf-hub` | `1.0` (blocking) | Model download from HuggingFace |

### MCP server

| Crate | Version | Purpose |
|---|---|---|
| `rmcp` | `=3.1.4` | MCP protocol implementation (exact pin — API stability) |
| `tokio` | `1` (full) | Async runtime for MCP server |
| `schemars` | `1` | JSON Schema generation for MCP tool parameters |

### File system / Git

| Crate | Version | Purpose |
|---|---|---|
| `git2` | `0.19` (no default features) | Git operations (remote detection, diff) |
| `walkdir` | `2` | Directory walking |
| `slug` | `0.1` | Slug generation for file paths |
| `ignore` | `0.4` | Gitignore-aware file walking (ripgrep engine) |

### Code parsing

| Crate | Version | Purpose |
|---|---|---|
| `tree-sitter` | `0.25` | AST parsing framework |
| `tree-sitter-language` | `0.1` | Language trait |
| `tree-sitter-rust` | `0.24` | Rust grammar |
| `tree-sitter-python` | `0.25` | Python grammar |
| `tree-sitter-go` | `0.25` | Go grammar |
| `tree-sitter-javascript` | `0.25` | JavaScript grammar |
| `tree-sitter-typescript` | `0.23` | TypeScript + TSX grammar |
| `tree-sitter-bash` | `0.25` | Bash grammar |
| `globset` | `0.4` | Glob pattern matching for index allow/deny |

### Database

| Crate | Version | Purpose |
|---|---|---|
| `rusqlite` | `0.40` (bundled) | SQLite bindings (bundled = no system SQLite needed) |
| `sqlite-vec` | `0.1` | Vector search extension for SQLite |

### Utilities

| Crate | Version | Purpose |
|---|---|---|
| `uuid` | `1` (v4, v5) | UUID generation |
| `sha2` | `0.10` | SHA-256 for content hashing and checksums |
| `chrono` | `0.4` (serde) | Timestamps |
| `anyhow` | `1` | Top-level error handling (CLI only) |
| `thiserror` | `2` | Typed error enums (library) |
| `tracing` | `0.1` | Structured logging |
| `tracing-subscriber` | `0.3` (env-filter) | Log subscriber setup |
| `zerocopy` | `0.8` | Zero-copy byte conversion for embeddings |
| `ndarray` | `0.16` | N-dimensional arrays for ONNX I/O |

### Self-update

| Crate | Version | Purpose |
|---|---|---|
| `ureq` | `3` (rustls, json) | HTTP client for GitHub API |
| `regex` | `1.13.1` | Version string parsing |
| `aho-corasick` | `1.1.5` | Multi-pattern string search (auto-link) |
| `fs2` | `0.4.3` | File locking for model cache |

### Platform-specific

| Crate | Version | Platform | Purpose |
|---|---|---|---|
| `libc` | `0.2` | Unix only | Stderr suppression during ONNX init |

### Dev dependencies

| Crate | Version | Purpose |
|---|---|---|
| `tempfile` | `3` | Temporary directories in tests |
| `pretty_assertions` | `1` | Readable assertion failures |
| `filetime` | `0.2` | File modification time in tests |
| `rmcp` | `=3.1.4` (client) | MCP client for subprocess tests |

## Build decisions

- **`rusqlite` with `bundled` feature** — avoids system SQLite dependency. Adds ~500 KB to compile time but simplifies deployment.
- **`git2` with no default features** — disables SSH and HTTPS support. CogZ only needs local repo discovery and diff, not remote operations.
- **`ort` with `load-dynamic` + `ndarray`** — loads ONNX Runtime as a shared library at runtime rather than linking it at build time. Keeps the binary small and allows runtime upgrades.
- **`rmcp` exact pin** — the MCP protocol implementation is still pre-1.0; exact pins prevent surprise breaking changes.

## Adding a dependency

1. Check if the functionality exists in the standard library or an existing dependency.
2. Check the version is at least 7 days old.
3. Use a caret pin for stable crates, exact pin for pre-release.
4. Add to `Cargo.toml` with a comment explaining the purpose.
5. Run `cargo build` and `cargo test` to verify compatibility.
6. Update this document.

## See also

- [Building](building.md) — build commands and release process
- [Conventions](conventions.md) — code patterns
