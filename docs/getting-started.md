# Getting Started

This guide explains the mental model behind CogZ and walks through a complete first-use scenario.

## The mental model

CogZ gives an AI coding agent three capabilities it normally lacks:

### 1. Memory

An agent that learns something about your repository on Monday should still know it on Friday. CogZ stores three types of persistent memory as Markdown files inside `.cogz/`:

- **Observations** — raw, unvalidated experience: bugs found, decisions made, patterns noticed. These are the agent's field notes. They can be promoted to rules through consolidation.
- **Rules** — validated directives the agent should follow: coding standards, design decisions, confirmed patterns. Rules are git-tracked and shared across a team.
- **Knowledge** — structured documentation about the codebase: architecture explanations, module responsibilities, trade-off rationale. Knowledge is human-readable, git-tracked, and meant to be read by both humans and agents.

All three are Markdown files with YAML frontmatter. They link to each other and to code entities (functions, classes, files) via `references` fields, forming a graph.

### 2. Context

When an agent starts a task, CogZ assembles a **context pack** — a scoped, ranked, token-budgeted collection of relevant observations, rules, knowledge, and code structures. The pack includes graph provenance: the agent can see *why* each piece was included and trace the path from the query to each result.

Three modes:
- **cold_start** — session start. Compact pack of recent rules and observations. No query needed.
- **task** — per-prompt. Ranked retrieval using the agent's query, with graph expansion from matched entities.
- **escalation** — when a task pack was insufficient. Wider retrieval with more results and more graph hops.

### 3. Cognition

CogZ continuously consolidates its memory:

- **Dedup** — on every insert, new entities are compared to existing ones via title match and embedding similarity. Duplicates are flagged.
- **Contradiction** — an NLI model checks new observations against existing ones and flags contradictions.
- **Promotion** — observations with enough supporting evidence (≥3 supporting observations) are promoted to rules.
- **Merge** — confirmed duplicate observations are merged: one survives, the other is marked `superseded` with edges redirected.

## The file-first invariant

Every entity is a Markdown file. The SQLite database is a derived index — disposable and fully rebuildable. If you delete the database, `cogz index` rebuilds it from the files and source code.

This means:
- Your knowledge is portable and version-controlled.
- You can edit entity files directly in your editor.
- The database can never be the source of truth — files always win.

## Walkthrough

### Step 1: Install

**Linux, macOS, or Windows (Git Bash):**
```bash
curl -fsSL https://raw.githubusercontent.com/balaianu/CogZ/master/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/balaianu/CogZ/master/install.ps1 | iex
```

This installs the `cogz` binary to `~/.local/bin/cogz` (or `~/.local/bin/cogz.exe` on Windows) and creates the model cache directory at `~/.local/share/cogz/models/`.

Verify:
```bash
cogz --version
```

### Step 2: Initialize

```bash
cd ~/your-project
cogz init
```

This creates:
```
.cogz/
  config.toml       # project configuration
  .gitignore        # ignores the DB, keeps knowledge/rules/observations
  knowledge/        # knowledge entries (subdirectories by category)
  rules/            # rule files
  observations/     # observation files (subdirectories by month)
```

By default, knowledge, rules, and observations are committed to git for team sharing. The database is gitignored. Use `cogz init --local-only` to keep everything local.

### Step 3: Index

```bash
cogz index
```

This does two things:
1. **Syncs entity files** — scans `.cogz/knowledge/`, `.cogz/rules/`, `.cogz/observations/` and syncs them to the SQLite database.
2. **Indexes source code** — parses source files with tree-sitter, extracting functions, classes, files, and modules as graph entities with structural edges (calls, imports, extends, contains).

On first run with `auto_download = true` (default), it downloads three ONNX models from HuggingFace (~436 MB total) for code embeddings, knowledge embeddings, and NLI contradiction detection. See [Configuration](configuration.md#embedding) for the model list and sizes.

To skip downloads and use FTS-only mode:
```bash
cogz index --no-download
```

### Step 4: Search

```bash
cogz search "authentication flow"
```

Search runs hybrid FTS5 + vector search with RRF (Reciprocal Rank Fusion) and graph expansion. Results include both knowledge entities (observations, rules, knowledge) and code entities (functions, classes, files).

Filter by type:
```bash
cogz search "auth" --entity-type function
cogz search "caching" --entity-type knowledge
```

Use the code model for code-focused queries:
```bash
cogz search "graph traversal BFS" --code
```

### Step 5: Assemble context

```bash
cogz context --mode task "implement rate limiting"
```

This produces a context pack — a JSON object with sections (each containing an entity's title and content), metadata (token count, search mode, dropped sources), and graph provenance.

For session start (no query needed):
```bash
cogz context --mode cold_start
```

### Step 6: Record an observation

Via MCP (preferred for agents):
```json
{
  "tool": "record_observation",
  "arguments": {
    "repo": "/home/user/my-project",
    "content": "The auth middleware checks JWT expiry before hitting the route handler",
    "title": "Auth middleware JWT expiry check",
    "references": ["src/middleware/auth.rs"]
  }
}
```

Or by creating a file directly:
```bash
cat > .cogz/observations/2026-09/my-observation.md << 'EOF'
---
id: 12345678-1234-1234-1234-123456789012
title: Auth middleware JWT expiry check
type: observation
status: active
created_at: 2026-09-07T09:00:00Z
updated_at: 2026-09-07T09:00:00Z
references: []
---

The auth middleware checks JWT expiry before hitting the route handler.
EOF
```

Then sync:
```bash
cogz index
```

### Step 7: Consolidate

```bash
cogz consolidate --dry-run    # see what would be consolidated
cogz consolidate              # run promotion and merge
```

Dedup and contradiction detection happen automatically on every insert. Promotion and merge are deferred — run them manually or via the `session_end` hook.

### Step 8: Health check

```bash
cogz doctor
```

Checks DB integrity, model availability, file-sync consistency, observation content edits, orphaned supersedes, near-duplicate knowledge, and vector dimension mismatches.

## FTS-only mode

If you don't want to download models, CogZ works in FTS-only mode. You lose vector search, embedding-based dedup, and contradiction detection, but everything else works: hooks, context packs, consolidation (title-based dedup only), doctor, prune.

```bash
cogz init
cogz index --no-download
cogz search "authentication"   # FTS-only search
cogz context --mode task "implement rate limiting"  # FTS-only context
```

## Next steps

- [Configuration](configuration.md) — tune search weights, token budgets, consolidation thresholds
- [CLI Reference](cli-reference.md) — every command and flag
- [MCP Tools](integration/mcp-tools.md) — integrate CogZ with your agent via MCP
- [Hooks](integration/hooks.md) — wire lifecycle events into your agent's session
