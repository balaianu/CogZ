---
id: c1e3f4a5-09cc-414e-831c-8f5710374d4a
title: Destructive operations must update DB before destroying the file
type: rule
status: active
created_at: "2026-09-03T12:13:00Z"
updated_at: "2026-09-03T12:13:00Z"
references: []
category: correctness
tags: ["prune", "deletion", "ordering", "file-first"]
confidence: 1.0
---

The file-first invariant says "write the file first, then sync the
DB." For destructive operations (prune, delete), the inverse
applies: **update the DB first, then destroy the file.**

**Why:** If the DB update fails, the file is still on disk and the
entity is in its pre-destruction state — a safe, retryable state.
If the file is destroyed first and the DB update fails, the content
is lost permanently (`cogz reset` + `cogz index` cannot rebuild from
a deleted file).

**Applies to:**
- `run_prune`: tombstone entity → delete embedding → delete file
- Any future code that deletes canonical entity files

**Reference pattern:** `run_prune` in `src/doctor/prune.rs`.
