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

### Code Parsing (Phase 8)

```toml
tree-sitter = "0.25"
tree-sitter-language = "0.1"
tree-sitter-rust = "0.24"
tree-sitter-python = "0.25"
ignore = "0.4"
globset = "0.4"
```

**tree-sitter 0.25** (latest: 0.26.13, published 2026-08-23)

We pin to 0.25.x (latest stable in the 0.25 line: 0.25.10, published
2025-09-22) rather than 0.26.x because:
- 0.26.x is a newer line (first release 2025-12-09) with less
  adoption time. The language crates may not all be compatible yet.
- 0.25.10 has 9.2M downloads — battle-tested.
- We can upgrade to 0.26 once all language crates confirm
  compatibility.

**tree-sitter-language 0.1** — language trait abstraction for
tree-sitter. Provides the `LanguageFn` type used to set parser
languages uniformly across language crates.

**tree-sitter-rust 0.24** (latest: 0.24.2) — Rust grammar for
tree-sitter. Compatible with tree-sitter 0.25.x.

**tree-sitter-python 0.25** (latest: 0.25.0) — Python grammar for
tree-sitter. Compatible with tree-sitter 0.25.x.

Language crates are pinned to versions compatible with tree-sitter
0.25.x. Each language crate depends on `tree-sitter ^0.25.8` or
similar. We verify compatibility at build time.

**ignore 0.4** (latest: 0.4.33) — `.gitignore` parsing and matching.
Uses the same crate as ripgrep. Handles gitignore syntax correctly
including negation, nested gitignores, and global gitignore. Used
for gitignore-aware source file scanning in Phase 8.

**globset 0.4** (latest: 0.4.20) — glob pattern matching for the
`[index].allow` configuration override. Although `globset` is
transitively available through `ignore`, we declare it explicitly
because we use it directly for allow-list matching.

### Embedding / ML

```toml
ort = { version = "=2.0.0-rc.13", default-features = false, features = ["load-dynamic", "ndarray"] }
tokenizers = "0.21"
ndarray = "0.16"
```

**ort 2.0.0-rc.13** (published 2026-07-28)

ONNX Runtime Rust wrapper. Supports ONNX Runtime 1.28 API. We pin
to rc.13 because:
- It supports ORT 1.27+ which matches FastEmbed's runtime version,
  ensuring comparable inference performance.
- rc.13 is over a month old, passing the 7-day rule.
- The ort docs explicitly say "this version is production-ready
  (just not API stable)."

Features:
- `load-dynamic` — loads ONNX Runtime at runtime via `dlopen()`
  instead of linking at build time. Avoids the `download-binaries`
  feature's build-time TLS dependency (openssl-sys via ureq/native-tls),
  which requires `pkg-config` and OpenSSL dev headers on the build
  machine. CogZ auto-discovers or downloads the ORT shared library
  to `~/.local/share/cogz/lib/libonnxruntime.so` at runtime. This
  aligns with the graceful degradation design: if the library isn't
  found, the system falls back to FTS-only search.
- `ndarray` — enables ndarray-based tensor operations.

We use `=2.0.0-rc.13` (exact pin) because pre-release versions can
have breaking changes between release candidates. We upgrade
deliberately, not via floating ranges.

**ndarray 0.16** — required by ort for tensor creation and
extraction. Pinned to match ort's internal ndarray version.

**tokenizers 0.21** — HuggingFace tokenizers for text tokenization
before embedding. Used by the ONNX embedding pipeline.

### MCP Server

```toml
rmcp = { version = "=3.1.4", features = ["transport-io"] }
tokio = { version = "1", features = ["full"] }
schemars = "1"
```

**rmcp 3.1.4** (published 2026-08-20, 341K downloads)

Official Rust MCP SDK. Implements the MCP 2026-07-28 spec. The SDK
went through 2.x and 3.x major cycles since the initial docs were
written. We pin to 3.1.4 with an exact pin (`=3.1.4`) because the
SDK is pre-1.0 and can have breaking changes between minor versions.
3.1.4 is 11+ days old as of our adoption, satisfying the 7-day rule.
Features:
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

**Dev-dependency:** `rmcp` is also listed in `[dev-dependencies]` with
the `client` feature enabled, for integration tests that use
`tokio::io::duplex` to connect a test client to the server.

### File System / Git

```toml
walkdir = "2"
git2 = { version = "0.19", default-features = false }
```

**walkdir 2** — recursive directory traversal for scanning `.cogz/`
and source files.

**git2 0.19** — libgit2 bindings for git diff and remote URL
detection (project name autodetection on `cogz init`). We disable
default features to avoid pulling in OpenSSL (hf-hub uses reqwest
with rustls-tls internally). libgit2 is bundled via the `bundled`
feature which is enabled by default in recent versions.

### Utilities

```toml
uuid = { version = "1", features = ["v4"] }
sha2 = "0.10"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
slug = "0.1"
zerocopy = "0.8"
```

| Crate | Purpose |
|---|---|
| `uuid` | UUID v4/v5 generation for entity IDs (v4 for file-backed, v5 deterministic for code) |
| `sha2` | SHA-256 content hashing for change detection |
| `chrono` | Timestamps (ISO 8601, serde-compatible) |
| `anyhow` | Application-level error handling (CLI, MCP handlers) |
| `thiserror` | Library-level error types (storage, embed, search) |
| `tracing` | Structured logging |
| `tracing-subscriber` | Log output configuration with env-filter |
| `slug` | Slug generation for file naming (knowledge, rules) |
| `zerocopy` | Zero-copy byte conversion for sqlite-vec vector I/O |
| `ndarray` | Tensor operations for ONNX embedding (pinned to match ort) |

### HuggingFace Hub client (Phase 12 — model download)

```toml
hf-hub = "1.0"
```

**hf-hub 1.0** — official Rust client for the Hugging Face Hub API,
the Rust equivalent of Python's `huggingface_hub`. Replaces the
earlier `reqwest` plan: `hf-hub` handles the HF API, content-addressed
caching, on-disk locking (concurrent fetch deduplication), and retry
logic. Synchronous API used because model download happens in a
synchronous context (first `cogz index` or `cogz models download`).

**Why hf-hub over raw reqwest:** the HF cache layout
(`models--{org}/{model}/blobs/`, `refs/main`, `snapshots/`) is
non-trivial. `hf-hub` manages it correctly and is compatible with
Python `huggingface_hub` caches. Raw `reqwest` would require
reimplementing cache layout, etag validation, and concurrent-access
locking — all of which `hf-hub` already handles.

**Published:** 2023-07-19 (v1.0.0 released 2026-07-10). 15.8M total
downloads, 515 dependents. Satisfies the 7-day rule.

**Partial download cleanup:** `hf-hub` uses retry logic (not resumable
downloads) and on-disk locking, which is more robust than Python's
`huggingface_hub`. However, the same cache layout means `.incomplete`
files and empty `refs/main` files can accumulate from interrupted
downloads or concurrent Python `huggingface_hub` usage in the same
cache directory. CogZ implements a defensive `clean_broken_cache()`
in `src/embed/download.rs` that runs before model loading:
- Removes `.incomplete` files older than 1 hour (preserves active
  downloads — 1 hour is conservative; a 400 MB model downloads in
  minutes on any reasonable connection)
- Removes empty `refs/main` files (0 bytes — blocks model loading)
This mirrors the Mnemos fix (`_clean_broken_cache` in `embed.py`)
adapted to Rust.

### Self-update HTTP client (Phase 12 — `cogz update`)

```toml
ureq = { version = "3", features = ["rustls", "json"] }
```

**ureq 3.x** — minimal HTTP client for the GitHub releases API.
Used by `cogz update` to check the latest release version, download
the new binary, and verify the SHA-256 checksum. `rustls` feature
avoids OpenSSL system dependency. `json` feature enables
`read_json()` for parsing the GitHub API response.

**Why ureq over reqwest:** `cogz update` is a one-shot CLI command,
not an async server. ureq's synchronous API is simpler and adds
fewer dependencies. reqwest is already pulled in transitively by
hf-hub, but ureq keeps the `update` module self-contained and
testable without the tokio runtime.

**Published:** v3.4.0 released 2026-09-15. Satisfies the 7-day rule.

---

## Dev Dependencies

```toml
[dev-dependencies]
tempfile = "3"
pretty_assertions = "1"
filetime = "0.2"
rmcp = { version = "=3.1.4", features = ["transport-io", "client"] }
```

| Crate | Purpose |
|---|---|
| `tempfile` | Temp directories and files for tests |
| `pretty_assertions` | Readable diff output for test assertions |
| `filetime` | Setting file modification times in download cleanup tests |
| `rmcp` | Client-side MCP transport for server integration tests |

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
# CLI
clap = { version = "4", features = ["derive"] }

# Config / Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Embedding / ML
ort = { version = "=2.0.0-rc.10", default-features = false, features = ["load-dynamic", "ndarray"] }
tokenizers = "0.21"
hf-hub = { version = "1.0", features = ["blocking"] }

# MCP server
rmcp = { version = "=3.1.4", features = ["transport-io"] }
tokio = { version = "1", features = ["full"] }
schemars = "1"

# File system / Git
git2 = { version = "0.19", default-features = false }
walkdir = "2"
slug = "0.1"
ignore = "0.4"

# Code parsing (Phase 8)
tree-sitter = "0.25"
tree-sitter-language = "0.1"
tree-sitter-rust = "0.24"
tree-sitter-python = "0.25"
globset = "0.4"

# Database
rusqlite = { version = "0.40", features = ["bundled"] }
sqlite-vec = "0.1"

# Utilities
uuid = { version = "1", features = ["v4", "v5"] }
sha2 = "0.10"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
zerocopy = "0.8"
ndarray = "0.16"

# Self-update (cogz update)
ureq = { version = "3", features = ["rustls", "json"] }

[dev-dependencies]
tempfile = "3"
pretty_assertions = "1"
filetime = "0.2"
rmcp = { version = "=3.1.4", features = ["transport-io", "client"] }

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
| Code Parsing | tree-sitter, tree-sitter-language, tree-sitter-rust, tree-sitter-python, ignore, globset | 6 |
| Embedding/ML | ort, tokenizers, ndarray, hf-hub | 4 |
| MCP server | rmcp, tokio, schemars | 3 |
| File system/Git | git2, walkdir, slug | 3 |
| Utilities | uuid, sha2, chrono, anyhow, thiserror, tracing, tracing-subscriber, zerocopy | 8 |
| Self-update | ureq | 1 |
| Dev | tempfile, pretty_assertions, filetime, rmcp (client feature) | 4 |
| **Total** | | **35** |

35 direct dependencies (Phase 12 scope). Transitive count is higher
but manageable. The binary is ~22 MB with static linking.
