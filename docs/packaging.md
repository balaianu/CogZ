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
  BAAI/bge-small-en-v1.5/         ~64 MB (code + knowledge embeddings)
  cross-encoder/nli-deberta-v3-xsmall/  ~87 MB (contradiction detection, optional)
```

Total model cache: ~151 MB for full capability, ~64 MB without NLI.

The default model is Qdrant's graph-optimized bge-small-en-v1.5 (384d,
INT8). It's used for both code and knowledge embeddings, keeping the
cache small and inference fast. CodeRankEmbed (768d, code-specific) and bge-base (768d) are
bge-base (768d, higher quality) are available as config alternatives.

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
downloads — a 64 MB model downloads in seconds on any reasonable
connection.

### ONNX Runtime

CogZ uses ONNX Runtime for model inference. The `ort` crate is
configured with `load-dynamic`, meaning the ONNX Runtime shared
library is loaded at runtime (not linked at build time). This avoids
build-time TLS dependencies and allows graceful degradation.

**Runtime discovery order:**
1. `ORT_DYLIB_PATH` environment variable
2. `~/.local/share/cogz/lib/libonnxruntime.so` (CogZ-managed)
3. System library paths (`/usr/lib/x86_64-linux-gnu/libonnxruntime.so`, etc.)

If no library is found, CogZ downloads ONNX Runtime 1.27.0 (CPU-only,
~23 MB) from GitHub releases and installs it to
`~/.local/share/cogz/lib/libonnxruntime.so`.

When no runtime is available and download fails, CogZ falls back to
FTS-only search (graceful degradation).

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
1. Downloads the code embedding model (~64 MB) if not cached and `auto_download` is true
2. Parses the repo with tree-sitter (respecting .gitignore)
3. Extracts code entities (functions, classes, files, modules)
4. Builds structural edges (calls, imports, extends)
5. Scans `.cogz/knowledge/` and `.cogz/rules/` for entity files
6. Embeds knowledge entities inline (skipped if model unavailable — FTS-only)
7. Builds FTS index
8. Creates `.cogz/cogz.db`
9. Spawns a background process to embed code entities (returns immediately)

Use `cogz index --no-download` to skip model download and operate in
FTS-only mode. Use `cogz models download` to pre-fetch models before
indexing.

Foreground index takes ~3 minutes on this repo (~1000 code entities).
Code embedding continues in the background for ~25 minutes. Search
works immediately after the foreground command returns — vector
results appear as embeddings are stored.

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
code_model = "BAAI/bge-small-en-v1.5"
knowledge_model = "BAAI/bge-small-en-v1.5"
dimension = 384
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

CogZ integrates with any agent system that supports command-based
lifecycle hooks (Devin, Claude Code, and compatible systems). The
binary is the hook handler — no wrapper scripts needed.

#### `cogz capture-event` flags

| Flag | Purpose |
|------|---------|
| `--hook-json` | Wrap output as `{"hookSpecificOutput": {...}}` for agent injection. Silent skip if no `.cogz/` found. Reads stdin for hook payload fields. |
| `--fts-only` | Skip ONNX model loading. Uses FTS-only search (~0.5s vs ~40s with models). Recommended for hooks. |
| `--repo <PATH>` | Repository root. Defaults to current directory. |
| `--prompt <TEXT>` | Prompt text (for `prompt_submit`). Also read from stdin. |
| `--tool-name <NAME>` | Tool name (for `post_tool_use`). Also read from stdin. |
| `--file-path <PATH>` | Saved file path (for `file_save`). Also read from stdin. |

#### Supported events

| Event | What CogZ does |
|-------|---------------|
| `session_start` | Records event, assembles cold-start context pack, prints it for agent injection |
| `prompt_submit` | Records event, assembles task-scoped context pack from the prompt, prints it |
| `post_tool_use` | Records event, optionally records an observation when tool_name and tool_result are available |
| `file_save` | Records event, triggers incremental code reindex for source files (.rs, .py) |
| `session_end` | Records event, runs consolidation (promotion + merge) |
| `stop` | Records event (no side effects) |

#### Devin / Claude Code configuration

Add entries to your agent's hook config. The `--hook-json` flag makes
`cogz capture-event` read stdin for event fields and output JSON in
the format these systems expect:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event session_start --hook-json --fts-only",
            "timeout": 10
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event prompt_submit --hook-json --fts-only",
            "timeout": 10
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event post_tool_use --hook-json --fts-only",
            "timeout": 10
          }
        ]
      },
      {
        "matcher": "edit|write|notebook_edit",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event file_save --hook-json --fts-only",
            "timeout": 20
          }
        ]
      }
    ],
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event stop --hook-json --fts-only",
            "timeout": 5
          }
        ]
      }
    ],
    "SessionEnd": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event session_end --hook-json --fts-only",
            "timeout": 30
          }
        ]
      }
    ],
    "PostCompaction": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cogz capture-event session_start --hook-json --fts-only",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

#### How it works

When `--hook-json` is set:
1. CogZ reads stdin for the agent's JSON payload (prompt, tool_name, tool_response, etc.)
2. If no `.cogz/` directory exists in the current repo, prints `{}` and exits (silent skip — safe for non-CogZ projects)
3. For `session_start` and `prompt_submit`: assembles a context pack and wraps it in `{"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}`
4. For other events: records the event, prints `{}`

When `--fts-only` is set:
- ONNX model loading is skipped entirely (no ~40s startup)
- Context assembly uses FTS-only search (fast, ~0.5s)
- The MCP server (which keeps models warm) is the preferred path for vector search

Hooks are lightweight — each call is a fresh process that opens the
DB, does its work, and exits. No daemon, no long-running process.

---

## Multi-User Collaboration

CogZ is designed for multi-user collaboration through a single Git
repo. The `.cogz/` directory (knowledge, rules, observations, config)
is tracked by Git. The SQLite database is gitignored — each machine
derives its own DB from the shared files.

### How it works

1. User A writes knowledge via `cogz` MCP tools or manual file edits
2. User A commits and pushes the `.cogz/*.md` files
3. User B pulls — Git merges the Markdown files (plain text, no
   conflicts in normal use)
4. User B's local DB is now stale — the files are current but the
   DB doesn't know about the new entries yet
5. User B runs `cogz reindex` — incremental sync picks up the new
   and changed files, updates the DB

Code entities (functions, classes, files, modules) use deterministic
UUID v5 IDs derived from `{file_path}:{entity_type}:{qualified_name}`,
so every machine produces the same entities from the same source code.
Embeddings are deterministic given the same model — every machine
produces the same vectors.

### Recommended: Git hooks for automatic reindex

Add a `post-merge` hook to reindex automatically after `git pull`:

```bash
# .git/hooks/post-merge
#!/bin/sh
cogz reindex 2>/dev/null
```

Optionally, add `post-checkout` for branch switching:

```bash
# .git/hooks/post-checkout
#!/bin/sh
cogz reindex 2>/dev/null
```

The `2>/dev/null` suppresses errors so the hook is a silent no-op
when cogz isn't installed or no `.cogz/` directory exists. This
makes the hooks safe to share across team members who haven't
installed CogZ — they simply do nothing.

Git hooks are not tracked by the repo and must be set up per-user,
same as agent hooks. CogZ does not install them automatically — it
is agent-agnostic and should not modify a user's Git configuration.

### What not to share

- **`cogz.db`** — gitignored, local only. Sharing it via Git would
  add a large binary that changes on every reindex, cause unsolvable
  merge conflicts, and break the "files are canonical, DB is derived"
  invariant.
- **Observations** — gitignored by default. Observations are per-user
  session notes. Teams that want shared observations can remove the
  gitignore entry, but this is a team decision, not a CogZ default.

### Caveats

- **Consolidation is a coordinated operation.** If two users run
  `cogz consolidate` simultaneously, both might promote the same
  observation or merge the same duplicates, creating conflicting
  file changes. In multi-user setups, consolidation should be run
  by one designated person, or the `session_end` hook (which runs
  consolidation) should be disabled.
- **Not all Git operations trigger hooks.** `git reset --hard`,
  `git stash pop`, and direct file edits don't trigger `post-merge`.
  Run `cogz reindex` manually in these cases.
- **Embedding model divergence.** If users configure different
  embedding models, vector search results will differ. FTS results
  are identical. This is acceptable — vector search is a local
  optimization.
- **Forward references resolve on reindex.** If user A creates
  knowledge that references user B's knowledge (by UUID), the
  reference edge won't resolve until both files are in the working
  tree. `cogz reindex` handles this via the second-pass reference
  sync — all reference edges are retried after all files are synced.

### Future: lazy stamp-file check

A planned enhancement (post-Phase 12) will add a lazy check: on
`cogz search` or `cogz context`, CogZ compares a quick hash of
`.cogz/` file mtimes against a stored stamp. If different, it runs
`cogz reindex` before proceeding. This catches cases where the Git
hook wasn't installed or was bypassed, without adding latency to
the common case (stamp matches → proceed immediately).

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
  BAAI/bge-small-en-v1.5/
  BAAI/bge-small-en-v1.5/
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
