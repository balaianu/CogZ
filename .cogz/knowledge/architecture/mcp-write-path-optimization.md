---
id: a3b7c8d9-09cc-414e-831c-8f5710374d4a
title: MCP write path uses single-file sync for O(1) I/O
type: knowledge
status: stale
created_at: "2026-09-03T12:01:00Z"
updated_at: "2026-09-04T09:15:30.748875943+00:00"
references: []
category: architecture
tags: ["mcp", "performance", "sync", "hot-path"]
---

# MCP write path uses single-file sync for O(1) I/O

MCP write tools (`create_knowledge`, `create_rule`,
`record_observation`, `update_knowledge`) each modify a single
entity file. Originally, they called `sync_incremental` to
synchronize the file into the DB, which scanned and parsed every
Markdown entity file in `.cogz/` — an O(n) operation.

With 41 entity files, every MCP write incurred 41 file reads and
parses just to sync one file. As the knowledge base grows, this
scales linearly.

## The fix

Replaced `sync_incremental` with `sync_single_file` in:
- `src/mcp/helpers.rs` (`write_and_sync`)
- `src/mcp/update_knowledge.rs` (`update_knowledge_file`)

`sync_single_file` reads and parses only the target file, syncs the
entity row, and calls `sync_references` for frontmatter edges. This
is O(1) relative to the number of entity files.

## Measurement

With 41 entity files:
- Before: 41 file reads + 41 parses per MCP write
- After: 1 file read + 1 parse per MCP write
- Improvement: 41x reduction in filesystem I/O

## Why not always use sync_single_file?

`sync_single_file` handles one file. `sync_incremental` (and
`sync_all`) handle all files, including detecting deleted files
(marking entities stale) and discovering new files. The MCP write
path knows exactly which file it just wrote, so single-file sync is
correct. The `file_save` hook also uses single-file sync for the
same reason.

`sync_incremental` is still used by `cogz reindex` and `cogz index`
for full synchronization, where scanning all files is the intended
behavior.
