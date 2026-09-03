---
id: b8c2d3e4-09cc-414e-831c-8f5710374d4a
title: Single-file sync must include reference edge synchronization
type: knowledge
status: active
created_at: "2026-09-03T11:56:00Z"
updated_at: "2026-09-03T11:56:00Z"
references: []
category: gotchas
tags: ["sync", "references", "file-save", "edges"]
---

# Single-file sync must include reference edge synchronization

`sync_single_file` was created for the `file_save` hook and for MCP
write tools. It syncs the entity row (INSERT/UPDATE) but originally
did not call `sync_references` — the function that re-syncs
frontmatter-derived graph edges (`references`, `supports`,
`contradicts`, `derived_from`).

This meant editing a knowledge file's frontmatter references via a
hook-triggered sync would update the entity content but not the graph
edges. Graph expansion through those edges would miss the updated
connections.

## The fix

`sync_single_file` now calls `sync_references` after a successful
non-skipped entity sync. This ensures all four frontmatter-derived
edge types are updated on single-file changes.

## When this matters

- `file_save` hook: editing a `.cogz/` file's frontmatter references
- MCP write tools: `create_knowledge`, `create_rule`, `record_observation`
  with `references` in the frontmatter
- `update_knowledge`: changing the `references` field

## When it doesn't matter

Code entities (function, class, file, module) don't have
frontmatter — their edges come from tree-sitter analysis, not from
`sync_references`. The fix only affects file-backed entities
(observation, rule, knowledge).
