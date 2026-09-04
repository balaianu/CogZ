---
id: a9c1d2e3-09cc-414e-831c-8f5710374d4a
title: Single-file operations must use sync_single_file not sync_incremental
type: rule
status: stale
created_at: "2026-09-03T12:11:00Z"
updated_at: "2026-09-04T09:15:30.815722776+00:00"
references: []
category: performance
tags: ["sync", "performance", "mcp", "hot-path"]
confidence: 1
---

When code knows which single file was modified, it must use
`sync_single_file` — not `sync_incremental` (which scans all
entity files) or `sync_all` (which re-syncs everything).

**Applies to:**
- MCP write tools (`write_and_sync`, `update_knowledge_file`)
- `file_save` hook handler
- Any future code that syncs one known file

**Why:** `sync_incremental` reads and parses every `.cogz/` Markdown
file even when only one changed. With 41 files, that's 41x more
I/O than necessary. `sync_single_file` reads only the target file,
syncs the entity row, and calls `sync_references` for frontmatter
edges — O(1) relative to entity count.

**When to use sync_incremental:** `cogz reindex` and `cogz index`,
where scanning all files is the intended behavior (detecting
deletions, discovering new files).
