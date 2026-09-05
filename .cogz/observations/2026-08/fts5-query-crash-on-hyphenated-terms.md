---
id: c4d5e6f7-89ab-4cde-f012-345678900113
title: FTS5 query crash on hyphenated search terms
type: observation
status: stale
created_at: "2026-08-29T23:30:00Z"
updated_at: "2026-09-03T21:27:12.291624157+00:00"
references: []
source: agent
confidence: 0.9
tags: ["bug", "fts5", "search", "phase-8-audit"]
---

FTS5 treats `-` as a NOT operator. Search queries containing hyphens
(e.g. "tree-sitter", "error-handling", "real-time") would either crash
with "no such column: sitter" or produce wrong results.

**Root cause:** `fts_search` in `storage/query.rs` passed the raw
query string directly to FTS5 MATCH without escaping special
characters.

**Fix:** Wrap the query in double quotes for FTS5 phrase matching.
Internal double quotes are escaped by doubling them (`""`). This
treats the entire query as a phrase, disabling all FTS5 operators.

**Trade-off:** Phrase matching is less flexible than FTS5 query
syntax — users can't use boolean operators (AND, OR, NOT), prefix
wildcards (`*`), or column filters (`title:`). This is acceptable for
CogZ's use case (natural language queries from agents), but a future
improvement could parse the query and selectively escape only
unintended special characters.
