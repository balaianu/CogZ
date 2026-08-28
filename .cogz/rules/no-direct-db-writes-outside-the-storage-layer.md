---
id: 5a2bdf52-00e5-4ff6-8749-65282c5e048b
title: "No direct DB writes outside the storage layer"
type: rule
status: active
created_at: 2026-08-28T19:33:00Z
updated_at: 2026-08-28T19:33:00Z
references: ["a9d8f4cd-a22a-4e0c-a25a-418c92564dcd"]
confidence: 1.0
validation_count: 2
supporting_ids: []
---

All SQL writes (INSERT, UPDATE, DELETE) must go through functions
in `src/storage/`. No other module should construct or execute
write SQL directly.

**Why:** The storage layer owns the schema, the triggers (FTS5
sync), and the connection. Bypassing it risks:

1. FTS5 index desync (triggers only fire on standard DML)
2. Missing content_hash computation
3. Missing event recording
4. Lock contention from multiple connection paths

**Enforcement:** This is a code review invariant, not a compiler
check. Grep for `conn.execute` and `INSERT INTO` outside `src/storage/`
to verify.

**Exception:** Test code may write directly to test specific
trigger behavior or schema edge cases. Production code never does.
