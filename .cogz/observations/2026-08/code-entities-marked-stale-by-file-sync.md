---
id: 7e3a2f1b-8c4d-4e5b-9a2f-1d6c3b5a7e8f
title: Code entities incorrectly marked stale by file sync phase
type: observation
status: stale
created_at: "2026-08-30T13:45:00Z"
updated_at: "2026-09-04T09:15:30.421727866+00:00"
references: []
source: agent
confidence: 0.9
---

The `mark_deleted_as_stale` function in `files/sync.rs` used
`file_path IS NOT NULL` to find file-backed entities. But code
entities (function, class, file, module) also have `file_path` set
to their source file path. This caused all 654 code entities to be
marked stale during the file sync phase of `cogz reindex`, then
reactivated by the code indexing phase.

The fix: filter by entity type (`observation`, `rule`, `knowledge`)
instead of `file_path IS NOT NULL`. Code entities are managed by the
index layer, not the file sync layer.

This was a performance issue (654 unnecessary status updates per
reindex) and produced misleading output ("Stale: 654" during file
sync). The final state was correct because the code indexing phase
reactivated everything, but the intermediate state was wrong.
