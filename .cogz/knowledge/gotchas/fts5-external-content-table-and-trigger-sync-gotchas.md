---
id: 06dd8275-c7a2-4b4f-b5a4-455b762062e9
title: "FTS5 external content table and trigger sync gotchas"
type: knowledge
status: active
created_at: 2026-08-28T19:28:00Z
updated_at: 2026-08-28T19:28:00Z
references: []
category: gotchas
tags: ["fts5", "sqlite", "triggers", "sync", "gotcha"]
---

The FTS5 table uses external content mode (`content='entities'`).
Three triggers keep it in sync: `entities_fts_ai` (after insert),
`entities_fts_ad` (after delete), `entities_fts_au` (after update).

**Gotcha 1:** The triggers use `entities_fts(rowid, ...)` for
inserts and `entities_fts(entities_fts, rowid, ...)` with
`'delete'` for deletes/updates. The `'delete'` special command is
required for external content tables — without it, FST entries
accumulate as orphans.

**Gotcha 2:** The update trigger does delete-then-insert, not a
direct update. This is the FTS5-recommended pattern for external
content tables.

**Gotcha 3:** If you ever bypass the triggers (e.g., raw SQL
`INSERT INTO entities`), the FTS index won't update. All entity
writes must go through `storage::crud` functions, which use
standard `INSERT`/`UPDATE` statements that fire the triggers.

**Gotcha 4:** The FTS5 tokenizer is `porter unicode61`. The Porter
stemmer handles English word variations (running → run). If
non-English content is common, consider adding a separate FTS
table with a different tokenizer.
