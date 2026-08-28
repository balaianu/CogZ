# CogZ — Dependencies

This document defines the exact crate list, versions, and build
decisions for the Rust project. Versions are pinned to the latest
stable as of August 2026, with a minimum age of 7+ days to avoid
supply-chain risk from freshly published releases.

---

## Core Dependencies

### Database

```toml
[dependencies]
rusqlite = { version = "0.40", features = ["bundled"] }
sqlite-vec = "0.1"
```

**rusqlite 0.40** (latest: 0.40.2, published 2026-08-08)

Synchronous SQLite wrapper. Chosen over sqlx because:
- CogZ is not async — the MCP server uses tokio but DB operations
  are fast and synchronous. Async DB adds complexity without benefit.
- `bundled` feature statically links SQLite — no system dependency,
  consistent version across installs, works on any Linux.
- Simpler API than sqlx for embedded use. No compile-time query
  checking needed (our queries are dynamic, built at runtime).
- sqlx 0.9 requires a database at compile time for query checking,
  which complicates CI and development.

**sqlite-vec 0.1** (latest: 0.1.10-alpha.4, prerelease)

SQLite vector search extension. Registered via
`sqlite3_auto_extension`. Uses the same SQLite connection as rusqlite.
Pre-v1, but stable enough for our use case (basic vec0 virtual table
operations). We pin to the latest stable release, not the alpha.

Note: sqlite-vec is pre-v1 and may have breaking changes. We isolate
all vec0 operations in `storage/` so a migration is contained if
needed.

### CLI

```toml
clap = { version = "4", features = ["derive"] }
```

**clap 4** (latest stable, well-established)

Derive macros for CLI definition. Standard choice. Features:
- `derive` — struct-based command definition

### Config

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
```

**serde 1** — serialization framework. `derive` for struct derive.
**serde_json 1** — JSON for entity properties and event payloads.
**toml 0.8** — TOML parsing for config files.

### Code Parsing

```toml
tree-sitter = "0.25"
tree-sitter-rust = "0.23"
tree-sitter-python = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-go = "0.23"
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
tree-sitter-java = "0.23"
tree-sitter-php = "0.23"
tree-sitter-c-sharp = "0.23"
```

**tree-sitter 0.25** (latest: 0.26.13, published 2026-08-23)

We pin to 0.25.x (latest stable in the 0.25 line: 0.25.10, published
2025-09-22) rather than 0.26.x because:
- 0.26.x is a newer line (first release 2025-12-09) with less
  adoption time. The language crates may not all be compatible yet.
- 0.25.10 has 9.2M downloads — battle-tested.
- We can upgrade to 0.26 once all language crates confirm
  compatibility.

Language crates are pinned to versions compatible with tree-sitter
0.25.x. Each language crate depends on `tree-sitter ^0.25.8` or
similar. We verify compatibility at build time.

Note: tree-sitter-python 0.25.0 requires tree-sitter ^0.25.8, which
is compatible with our 0.25.x pin. If any language crate requires
0.26.x, we'll upgrade the whole stack together.

### Embedding / ML

```toml
ort = { version = "=2.0.0-rc.10", default-features = false, features = ["load-dynamic", "ndarray"] }
tokenizers = "0.21"
ndarray = "0.16"
```

**ort 2.0.0-rc.10** (latest: 2.0.0-rc.13, published 2026-07-28)

ONNX Runtime Rust wrapper. We pin to rc.10 (published 2025-06-01)
rather than rc.13 because:
- rc.10 has 4.4M downloads — widely used and tested.
- rc.13 is only a month old (2026-07-28). We follow the 7-day rule.
- The ort docs explicitly say "this version is production-ready
  (just not API stable)."

Features:
- `load-dynamic` — loads ONNX Runtime at runtime via `dlopen()`
  instead of linking at build time. Avoids the `download-binaries`
  feature's build-time TLS dependency (openssl-sys via ureq/native-tls),
  which requires `pkg-config` and OpenSSL dev headers on the build
  machine. The user must have `libonnxruntime.so` available at
  runtime (or set `ORT_DYLIB_PATH`). This aligns with the graceful
  degradation design: if the library isn't found, the system falls
  back to FTS-only search.
- `ndarray` — enables ndarray-based tensor operations.

We use `=2.0.0-rc.10` (exact pin) because pre-release versions can
have breaking changes between release candidates. We upgrade
deliberately, not via floating ranges.

**ndarray 0.16** — required by ort for tensor creation and
extraction. Pinned to match ort's internal ndarray version.

**tokenizers 0.21** — HuggingFace tokenizers for text tokenization
before embedding. Used by the ONNX embedding pipeline.

### MCP Server

```toml
rmcp = { version = "3.1.2", features = ["transport-io"] }
tokio = { version = "1", features = ["full"] }
schemars = "1"
```

**rmcp 3.1.2** (published 2026-08-07, 341K downloads)

Official Rust MCP SDK. Implements the MCP 2026-07-28 spec. The SDK
went through 2.x and 3.x major cycles since the initial docs were
written. We pin to 3.1.2 rather than 3.1.4 (2026-08-20, only 9 days
old) to stay conservative on the 7-day rule. Features:
- `server` (default) — server-side implementation, tool handler traits
- `macros` (default) — `#[tool]` and `#[tool_router]` attribute macros
- `transport-io` — stdio transport for `cogz mcp-stdio`

The SDK uses tokio async runtime. Our MCP tools are async handlers
that call into the synchronous storage layer via
`tokio::task::spawn_blocking`. The boundary is clean: async at the
protocol level, sync at the DB level.

**tokio 1** — async runtime. `full` features for stdio transport,
`spawn_blocking`, and concurrent request handling.

**schemars 1** — JSON Schema generation from Rust types. Used by
rmcp's `#[tool]` macro to generate `inputSchema` for tool definitions.
Derives `JsonSchema` on tool parameter structs alongside `serde::Deserialize`.

### File System / Git

```toml
walkdir = "2"
git2 = "0.19"
ignore = "0.4"
```

**walkdir 2** — recursive directory traversal for scanning `.cogz/`
and source files.

**git2 0.19** — libgit2 bindings for git diff and remote URL
detection (project name autodetection on `cogz init`).

**ignore 0.4** — `.gitignore` parsing and matching. Uses the same
crate as ripgrep. Handles gitignore syntax correctly including
negation, nested gitignores, and global gitignore.

### Utilities

```toml
uuid = { version = "1", features = ["v4"] }
sha2 = "0.10"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"
slug = "0.1"
zerocopy = "0.8"
```

| Crate | Purpose |
|---|---|
| `uuid` | UUID v4 generation for entity IDs |
| `sha2` | SHA-256 content hashing for change detection |
| `chrono` | Timestamps (ISO 8601, serde-compatible) |
| `anyhow` | Application-level error handling (CLI, MCP handlers) |
| `thiserror` | Library-level error types (storage, embed, search) |
| `tracing` | Structured logging |
| `tracing-subscriber` | Log output configuration |
| `slug` | Slug generation for file naming (knowledge, rules) |
| `zerocopy` | Zero-copy byte conversion for sqlite-vec vector I/O |

### HTTP (for model download)

```toml
reqwest = { version = "0.12", features = ["blocking"], default-features = false }
```

**reqwest 0.12** — HTTP client for downloading ONNX models from
HuggingFace. `blocking` feature because model download happens in a
synchronous context (first `cogz index`). `default-features = false`
to avoid pulling in unnecessary dependencies; we add only what's
needed.

---

## Dev Dependencies

```toml
[dev-dependencies]
tempfile = "3"
pretty_assertions = "1"
```

| Crate | Purpose |
|---|---|
| `tempfile` | Temp directories and files for tests |
| `pretty_assertions` | Readable diff output for test assertions |

---

## Build Decisions

### Static linking

- **SQLite**: `bundled` feature in rusqlite. Statically linked, no
  system SQLite dependency.
- **sqlite-vec**: Compiled from C source at build time via the `cc`
  crate. Statically linked.
- **ONNX Runtime**: `load-dynamic` feature in ort. Loaded at runtime
  via `dlopen()` — not bundled. The user must have
  `libonnxruntime.so` available (or set `ORT_DYLIB_PATH`). This
  avoids the build-time OpenSSL dependency and aligns with graceful
  degradation.
- **libgit2**: `git2` crate bundles libgit2 via the `bundled` feature
  (enabled by default in recent versions).

Result: the release binary has minimal runtime dependencies. Only
system libc. ONNX Runtime is loaded dynamically at runtime if
available; the system functions without it (FTS-only mode).

### MSRV (Minimum Supported Rust Version)

**Rust 1.88** — required by ort 2.0.0-rc.10. This is the highest MSRV
among our dependencies. We document this in the README and CI.

### Edition

**Rust 2024 edition** — latest stable edition, supported by all our
dependencies.

### Profile (release)

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
```

- `opt-level = 3` — maximum optimization
- `lto = "fat"` — cross-crate inlining, smaller binary
- `codegen-units = 1` — better optimization at the cost of compile time
- `strip = true` — strip debug symbols from release binary

This produces a small, fast binary at the cost of longer compile
time. Acceptable for releases.

### Profile (dev)

```toml
[profile.dev]
opt-level = 0
debug = true
```

Fast compile, debug symbols on. Default dev profile.

---

## Full Cargo.toml

```toml
[package]
name = "cogz"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
description = "Local-first, code-aware engineering cognition runtime"
license = "MIT"

[dependencies]
# Database
rusqlite = { version = "0.40", features = ["bundled"] }
sqlite-vec = "0.1"

# CLI
clap = { version = "4", features = ["derive"] }

# Config / Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Code parsing
tree-sitter = "0.25"
tree-sitter-rust = "0.23"
tree-sitter-python = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-go = "0.23"
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
tree-sitter-java = "0.23"
tree-sitter-php = "0.23"
tree-sitter-c-sharp = "0.23"

# Embedding / ML
ort = { version = "=2.0.0-rc.10", default-features = false, features = ["load-dynamic", "ndarray"] }
tokenizers = "0.21"
ndarray = "0.16"

# MCP server
rmcp = { version = "3.1.2", features = ["transport-io"] }
tokio = { version = "1", features = ["full"] }
schemars = "1"

# File system / Git
walkdir = "2"
git2 = "0.19"
ignore = "0.4"

# Utilities
uuid = { version = "1", features = ["v4"] }
sha2 = "0.10"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"
slug = "0.1"
zerocopy = "0.8"

# HTTP (model download)
reqwest = { version = "0.12", features = ["blocking"], default-features = false }

[dev-dependencies]
tempfile = "3"
pretty_assertions = "1"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true

[profile.dev]
opt-level = 0
debug = true
```

---

## Version Policy

- **Exact pins** for pre-release crates (ort) — pre-release versions
  can break between release candidates.
- **Caret pins** (`^`) for stable crates — allows patch and minor
  updates, prevents major version bumps.
- **No floating ranges** (`*`, `latest`) — every dependency has a
  bounded version.
- **Upgrade deliberately** — review changelog, test, then bump.
- **7-day rule** — don't use versions published less than 7 days ago.

---

## Dependency Tree Summary

| Category | Crates | Count |
|---|---|---|
| Database | rusqlite, sqlite-vec | 2 |
| CLI | clap | 1 |
| Config/Serialization | serde, serde_json, toml | 3 |
| Code parsing | tree-sitter + 10 language crates | 11 |
| Embedding/ML | ort, tokenizers | 2 |
| MCP server | rmcp, tokio, schemars | 3 |
| File system/Git | walkdir, git2, ignore | 3 |
| Utilities | uuid, sha2, chrono, anyhow, thiserror, tracing, tracing-subscriber, slug, zerocopy | 9 |
| HTTP | reqwest | 1 |
| Dev | tempfile, pretty_assertions | 2 |
| **Total** | | **37** |

37 direct dependencies. Transitive count will be higher but
manageable. The binary will be ~15-25 MB with static linking.
