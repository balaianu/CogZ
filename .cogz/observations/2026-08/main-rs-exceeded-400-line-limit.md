---
id: c4d5e6f7-89ab-4cde-f012-345678900112
title: main.rs exceeded 400-line limit — extracted commands.rs
type: observation
status: stale
created_at: "2026-08-29T23:25:00Z"
updated_at: "2026-09-04T12:59:23.347428584+00:00"
references: []
source: agent
confidence: 0.9
tags: ["refactor", "file-size", "phase-8-audit"]
---

`main.rs` was 417 lines, exceeding the repository's 400-line limit.
The file contained both the clap CLI definition (structs, enums, match
dispatch) and the implementation of `run_status`, `run_index`,
`run_reindex`, and `run_reset` handlers.

**Fix:** Extracted the four handler functions to a new `commands.rs`
module. `main.rs` is now 154 lines (CLI definition + dispatch).
`commands.rs` is 246 lines (handler implementations). Both well under
the 400-line limit.

The handlers use a shared `load_config` helper that reads the config
and returns `(Config, db_path)`, reducing duplication across the four
functions.
