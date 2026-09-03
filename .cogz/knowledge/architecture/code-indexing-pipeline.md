---
id: b3c4d5e6-f789-4abc-def0-123456789001
title: "Code indexing pipeline — scan, parse, sync, edges"
type: knowledge
status: stale
created_at: "2026-08-29T23:20:00Z"
updated_at: "2026-09-03T15:31:31.313491001+00:00"
references: []
category: architecture
tags: ["indexing", "tree-sitter", "code-entities", "phase-8"]
---

The code indexing pipeline runs as part of `cogz index` and
`cogz reindex`, after the file sync phase.

## Pipeline stages

1. **Scan** (`index/gitignore.rs`): Walk the repo with the `ignore`
   crate, respecting `.gitignore`. Exclude `.cogz/`. A second pass
   checks `[index].allow` glob patterns and adds matching files even
   if gitignored. Returns relative paths.

2. **Parse** (`index/tree_sitter.rs` + `tree_sitter/python.rs`):
   Tree-sitter parses each source file. Rust and Python are
   supported. Always emits a `file` entity. Extracts functions,
   classes (Rust: structs, enums, traits, impls; Python: classes),
   and modules. Each entity has `file_path`, `line_start`, `line_end`,
   `language`, `qualified_name`, `signature`, and `kind`.

3. **Sync** (`index/sync/mod.rs`): Entities are synchronized to the
   DB with deterministic UUID v5 IDs based on
   `{file_path}:{entity_type}:{qualified_name}`. Content hash change
   detection avoids unnecessary updates. Removed source files →
   entities marked `stale`. `created_at` is preserved on updates.

4. **Edges** (`index/code_graph/mod.rs` + `code_graph/python.rs`):
   Structural edges are built from the AST. Three edge types:
   `calls` (function → function), `imports` (module/file →
   module/file), `extends` (class → class). Name-to-UUID matching
   uses both qualified and simple name lookup.

## Key design decisions

- **Rust impl blocks** get qualified names prefixed with `impl`
  (e.g. `impl Point`, `impl Display for Point`) to avoid UUID
  collisions with the struct entity of the same type name.
- **Code entities are database-only** — no Markdown files on disk.
  They're rebuildable from source.
- **The `ignore` crate requires repository context** for gitignore
  behavior. Tests create a minimal `.git` directory.
- **Module splitting**: `tree_sitter.rs`, `code_graph/mod.rs`, and
  `sync/mod.rs` were split into submodules to stay under the 400-line
  file limit. Python extraction lives in `python.rs` submodules.
