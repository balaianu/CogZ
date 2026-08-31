# CogZ — Packaging, Distribution, Installation

This document defines how CogZ is packaged, distributed, installed,
updated, and uninstalled. The design leverages the single-binary
advantage of Rust while handling the realities of model downloads
and per-repo setup.

---

## What Gets Distributed

CogZ has three components, distributed differently:

| Component | Size | How distributed | When delivered |
|---|---|---|---|
| CogZ binary | ~10-20 MB | GitHub release artifact / package / cargo | At install time |
| ONNX models | ~100-400 MB each | Downloaded from HuggingFace on first run | At first `cogz index` |
| Tree-sitter grammars | Compiled into binary | Part of the binary | At install time |

The binary contains everything except the ML models. Tree-sitter
grammars for supported languages are compiled in as Rust crates — no
separate download, no dynamic loading. The ONNX models are too large
to bundle and are downloaded on demand.

### Supported languages (compiled in)

The binary includes tree-sitter grammars for:

- Python, JavaScript, TypeScript, Go, Rust, C, C++, Java, PHP, C#

Adding a language requires adding a crate dependency and rebuilding.
This is a compile-time decision, not a runtime one. The binary
supports a fixed set of languages — it does not download grammars
at runtime.

### Model cache

Models are downloaded to `~/.local/share/cogz/models/` on first use:

```
~/.local/share/cogz/models/
  nomic-ai/CodeRankEmbed-int8/   ~100 MB (code embeddings)
  BAAI/bge-base-en-v1.5/         ~400 MB (knowledge embeddings)
  nli-deberta-v3-xsmall/         ~280 MB (contradiction detection, optional)
```

Total model cache: ~780 MB for full capability, ~500 MB without NLI.

Download is lazy and automatic unless disabled:
- `cogz index` triggers code model download (if `auto_download = true`)
- First knowledge/observation embedding triggers knowledge model download
- First contradiction check triggers NLI model download
- Each model is downloaded only when first needed, not all upfront

**Disabling auto-download:**
- Config: `[embedding].auto_download = false` in `.cogz/config.toml`
- CLI: `cogz index --no-download` for a one-time opt-out
- When disabled, CogZ operates in FTS-only mode (graceful degradation)

**Manual download:**
- `cogz models download` — download all configured models
- `cogz models download --code` — download only the code model
- `cogz models download --knowledge` — download only the knowledge model
- `cogz models download --nli` — download only the NLI model
- `cogz models list` — show configured models and download status
- `cogz models clean` — remove `.incomplete` files and empty `refs/main`

Download is cached via `hf-hub` (content-addressed, on-disk locking).
If a model is already cached, it's not re-downloaded. `hf-hub` uses
retry logic on transient failures.

**Partial download cleanup:** before each model load, CogZ runs
`clean_broken_cache()` which removes `.incomplete` files older than
1 hour and empty `refs/main` files from the HF cache. This prevents
the disk-filling retry loop that affected Mnemos (Python
`huggingface_hub` leaves `.incomplete` files on interrupted
downloads). The 1-hour threshold preserves files from active
downloads — a 400 MB model downloads in minutes on any reasonable
connection.

---

## Distribution Channels

### Primary: GitHub Releases

The main distribution channel. Each release publishes:

- `cogz-x86_64-unknown-linux-gnu` — statically linked binary for Linux x86_64
- `cogz-aarch64-unknown-linux-gnu` — for ARM64 Linux (future, if needed)
- `SHA256SUMS` — checksums for verification
- `SHA256SUMS.sig` — GPG signature (optional, if a signing key is set up)

The binary is built with `RUSTFLAGS="-C target-feature=+crt-static"` for
static linking where possible. If some system libraries are needed
(SQLite is bundled via `libsqlite3-sys`), the binary has minimal
runtime dependencies.

### Secondary: Cargo

```bash
cargo install cogz
```

Installs from crates.io (or directly from git). This requires Rust
toolchain on the target machine — appropriate for developers, not
end users. The binary lands in `~/.cargo/bin/cogz`.

### Tertiary: Install script

```bash
curl -fsSL https://raw.githubusercontent.com/balaianu/CogZ/main/install.sh | bash
```

The script:
1. Detects architecture (x86_64, aarch64)
2. Downloads the correct binary from GitHub releases
3. Verifies checksum
4. Installs to `~/.local/bin/cogz` (or `/usr/local/bin/cogz` if run as root)
5. Creates `~/.local/share/cogz/models/` directory
6. Prints next steps (`cogz init` in a repo)

The script is signed and the checksum is verified. The user is
prompted before installation if `~/.local/bin` is not on PATH.

### Future: Package managers

- **AUR** (`cogz-bin` package) — for Arch users, wraps the GitHub binary
- **Homebrew tap** — for macOS users (if macOS support is added)
- **.deb package** — for Debian/Ubuntu, if demand warrants

These are not initial release targets. The binary + install script
covers the primary use case (Linux, single user). Package manager
support can be added later without changing the build process.

---

## Installation

### User-level install (no sudo)

```bash
# Via install script
curl -fsSL https://raw.githubusercontent.com/balaianu/CogZ/main/install.sh | bash

# Or manual
wget https://github.com/balaianu/CogZ/releases/latest/download/cogz-x86_64-unknown-linux-gnu
chmod +x cogz-x86_64-unknown-linux-gnu
mv cogz-x86_64-unknown-linux-gnu ~/.local/bin/cogz
```

Result:
```
~/.local/bin/cogz                          — the binary
~/.local/share/cogz/models/                — model cache (empty, populated on first use)
```

No system files touched. No sudo required. Fully user-space.

### System-level install (with sudo, optional)

```bash
sudo mv cogz /usr/local/bin/cogz
```

For multi-user systems. The binary is shared, but each user has their
own model cache at `~/.local/share/cogz/models/`. Per-repo `.cogz/`
directories are always per-user (they live in the repo).

### Per-repo initialization

After installing the binary, the user initializes CogZ in a repo:

```bash
cd ~/projects/my-project
cogz init
```

This creates:
```
my-project/
  .cogz/
    config.toml          — generated with defaults, project name autodetected
    knowledge/           — empty, ready for knowledge files
    rules/               — empty, ready for rule files
    observations/        — empty (gitignored)
    .gitignore           — generated: ignores observations/ and cogz.db
```

The `.gitignore` is created by `cogz init` with:

```gitignore
# CogZ — generated by cogz init
.cogz/observations/
.cogz/cogz.db
.cogz/cogz.db-wal
.cogz/cogz.db-shm
```

Knowledge and rules are NOT gitignored — they're meant to be
version-tracked.

### First index

```bash
cogz index
```

This:
1. Downloads the code embedding model (~100 MB) if not cached and `auto_download` is true
2. Parses the repo with tree-sitter (respecting .gitignore)
3. Extracts code entities (functions, classes, files, modules)
4. Builds structural edges (calls, imports, extends)
5. Scans `.cogz/knowledge/` and `.cogz/rules/` for entity files
6. Downloads the knowledge embedding model (~400 MB) if knowledge files exist and `auto_download` is true
7. Generates embeddings for all entities (skipped if models unavailable — FTS-only)
8. Builds FTS index
9. Creates `.cogz/cogz.db`

Use `cogz index --no-download` to skip model download and operate in
FTS-only mode. Use `cogz models download` to pre-fetch models before
indexing.

First index takes a few minutes on a large repo (model download +
parsing + embedding). Subsequent indexes are incremental (only
changed files re-embedded).

---

## Updates

### Binary update

```bash
cogz update
```

Or manually via the install script:

```bash
curl -fsSL https://raw.githubusercontent.com/balaianu/CogZ/main/install.sh | bash
```

The `cogz update` command:
1. Checks the latest release version from GitHub API
2. Compares with current version (compiled-in `CARGO_PKG_VERSION`)
3. If newer: downloads the new binary, verifies checksum, replaces itself
4. If same: prints "CogZ is up to date (vX.Y.Z)"

The update is atomic: the new binary is downloaded to a temp file,
verified, then renamed over the old one. If the download or
verification fails, the old binary is untouched.

### Database migrations

When the binary is updated, the schema version in `.cogz/cogz.db`
might be older than what the new binary expects. On first run after
update, CogZ checks the schema version and runs migrations if needed.

Migrations are forward-only and non-destructive:
- Add columns with defaults
- Create new indexes
- Backfill data if needed

If a migration fails, CogZ refuses to start and prints instructions
to rebuild the DB from files: `cogz reindex --force` (drops and
rebuilds the DB from `.cogz/` files + code). Since the DB is derived
and disposable, this is always safe.

### Model updates

Models are pinned in config.toml. Changing a model version requires
updating config and re-indexing (embeddings are model-specific).

```toml
[embedding]
code_model = "nomic-ai/CodeRankEmbed-int8"
knowledge_model = "BAAI/bge-base-en-v1.5"
```

If a model is updated in config, `cogz index` detects the mismatch
(model name + hash differs from cached model) and re-downloads.
All embeddings are regenerated for the new model.

Model updates are not automatic. The user changes the config, runs
`cogz index`, and the system handles the rest. This is intentional —
model changes affect all embeddings and should be a conscious
decision.

---

## Uninstallation

### Remove from a repo

Two levels of reset:

**`cogz reset` — drops the DB only:**
```bash
cd ~/projects/my-project
cogz reset
```

Removes:
- `.cogz/cogz.db` and related WAL/SHM files

Keeps everything else. This is the safe reset — use it when the DB is
corrupted or you want a clean reindex. Run `cogz index` to rebuild
from files + code.

**`cogz reset --purge` — removes CogZ runtime artifacts:**
```bash
cd ~/projects/my-project
cogz reset --purge
```

Removes:
- `.cogz/cogz.db` and related WAL/SHM files
- `.cogz/observations/` (gitignored, local-only agent experience)
- `.cogz/.gitignore` (the one generated by `cogz init`)

**Does NOT remove:**
- `.cogz/config.toml` — user configuration, may have custom settings
- `.cogz/knowledge/` — human-authored, git-tracked documentation
- `.cogz/rules/` — validated, git-tracked knowledge

These are never removed by CogZ. They are the user's content. If the
user wants to remove them, they do it manually:

```bash
rm -rf .cogz/
```

This is intentional. CogZ does not delete human-authored content.
Knowledge and rules may represent hours of work. A tool command
should never destroy them, even with a confirmation prompt —
confirmation prompts get clicked through, and the loss is
irreversible.

### Remove the binary

```bash
rm ~/.local/bin/cogz
```

Or if installed system-wide:

```bash
sudo rm /usr/local/bin/cogz
```

### Remove the model cache

```bash
rm -rf ~/.local/share/cogz/
```

This removes all downloaded models (~780 MB). No other data lives
here.

### Full uninstall

```bash
# 1. Purge runtime artifacts from each repo (keeps knowledge/rules/config)
cd ~/projects/my-project && cogz reset --purge
cd ~/projects/other-project && cogz reset --purge

# 2. Remove the binary
rm ~/.local/bin/cogz

# 3. Remove model cache
rm -rf ~/.local/share/cogz/
```

Three steps. Knowledge, rules, and config remain in each repo — the
user removes those manually if they want (`rm -rf .cogz/`). No
orphaned runtime files. No system-level changes to undo.

---

## Usage

### Daily workflow

The user doesn't interact with CogZ directly day-to-day. The agent
does, via MCP tools. The user's touchpoints are:

**Setting up a new repo (one-time):**
```bash
cd ~/projects/new-project
cogz init
cogz index
```

**After significant code changes (occasional):**
```bash
cogz reindex        # re-index changed files only
```

**Checking health (rare):**
```bash
cogz status         # DB size, entity counts, model status
cogz doctor         # health check, reports issues
cogz doctor --prune-observations  # report prunable observations (dry-run)
cogz doctor --prune-observations --confirm  # prune, preserve tombstones
```

**Manual knowledge management (optional):**
```bash
# Write a knowledge file directly
vim .cogz/knowledge/architecture/search-design.md

# Or via CLI
cogz create-knowledge --title "Search Design" --category architecture
# (opens $EDITOR with a template, saves to .cogz/knowledge/architecture/)

# Reindex to pick up the new file
cogz reindex
```

### Agent integration

The agent interacts with CogZ via MCP. The MCP server is configured
in the agent's config file:

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"],
      "env": {
        "COGZ_REPO": "/home/andy/projects/my-project"
      }
    }
  }
}
```

Or, if running from within the repo directory, `COGZ_REPO` is
auto-detected (same as `cogz init` does).

The MCP server runs as a subprocess of the agent. It opens the
per-repo DB, loads models lazily, and serves tool calls. When the
agent session ends, the MCP server exits and models are unloaded.

### Hook integration

Hooks capture lifecycle events and inject context:

```json
{
  "hooks": {
    "session_start": "cogz capture-event session_start",
    "prompt_submit": "cogz capture-event prompt_submit --prompt-file $PROMPT_FILE"
  }
}
```

The hook scripts call `cogz capture-event`, which:
1. Records the event in the DB
2. For `session_start`: generates a cold_start context pack and prints it to stdout (the agent reads this as injected context)
3. For `prompt_submit`: generates a task context pack and prints it

Hooks are lightweight — they call the binary, which does the work.
No daemon, no long-running process.

---

## Versioning

### Semantic versioning

CogZ follows semver: `MAJOR.MINOR.PATCH`

- **MAJOR**: schema changes that require DB rebuild or migration
- **MINOR**: new features, new MCP tools, new supported languages
- **PATCH**: bug fixes, performance improvements, model updates

### Version reporting

```bash
cogz --version       # cogz 0.1.0
cogz status          # includes version, schema version, model versions
```

The schema version is stored in the DB (`PRAGMA user_version`).
The binary knows its expected schema version (compiled in). On
startup, if DB schema < binary schema, migrations run automatically.

---

## Build Process

### Development build

```bash
cd ~/dev/personal/CogZ
cargo build --release
```

The release binary is at `target/release/cogz`. Copy to
`~/.local/bin/cogz` for testing.

### Release build

```bash
cargo build --release --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/cogz cogz-x86_64-unknown-linux-gnu
sha256sum cogz-x86_64-unknown-linux-gnu > SHA256SUMS
```

The release binary should be built with:
- `--release` for optimizations
- Static SQLite (`libsqlite3-sys` bundled, no system SQLite dependency)
- Static ONNX Runtime if possible (or bundled via `ort`)

### CI (future)

GitHub Actions workflow:
1. On tag push (`v*`): build release binary for target platforms
2. Run tests
3. Create GitHub release with binary artifact + checksums
4. Update install script to point to latest release

---

## Directory Summary

After full installation and setup, the filesystem looks like:

```
~/.local/bin/cogz                              — binary (10-20 MB)
~/.local/share/cogz/models/                    — model cache (~780 MB)
  nomic-ai/CodeRankEmbed-int8/
  BAAI/bge-base-en-v1.5/
  nli-deberta-v3-xsmall/

~/projects/my-project/                         — a repo using CogZ
  .cogz/
    config.toml                                — git-tracked
    knowledge/                                 — git-tracked
      architecture/
        search-design.md
    rules/                                     — git-tracked
      search-rules.md
    observations/                              — gitignored
      2026-08/
        <uuid>.md
    cogz.db                                    — gitignored
    .gitignore                                 — git-tracked (generated by cogz init)
```

Nothing in `/etc/`, `/usr/`, or system directories (unless installed
system-wide). Nothing in systemd. No daemons. No background services.
The binary runs on demand — invoked by the user, the agent, or hook
scripts — and exits when done.
