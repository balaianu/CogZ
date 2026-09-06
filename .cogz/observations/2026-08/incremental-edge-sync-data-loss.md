---
id: 63d8e153-f773-4975-9674-1ca830f1110d
title: Incremental reindex lost all structural edges (sync_code_edges global delete)
type: observation
status: stale
created_at: "2026-08-31T10:00:00Z"
updated_at: "2026-09-05T12:17:25.597153443+00:00"
references: []
source: agent
confidence: 0.9
---

The full-scan `sync_code_edges` function deletes ALL structural edges
(calls, imports, extends) before re-inserting from the parsed source
files. When the Phase 10 incremental reindex path called this function
with only the changed files, all edges from unchanged files were lost.

Reproduced: fresh index produced 1145 edges. Modifying one file and
running `cogz reindex` dropped edges to 14 (only the changed file's
edges survived).

Root cause: `sync_code_edges` was designed for full scans where
delete-all-then-rebuild is correct. The incremental path reused it
without considering that only changed files' edges would be re-inserted.

Fix: added `sync_code_edges_incremental` in
`src/index/code_graph/incremental.rs` which builds the name-to-UUID
map from ALL code entities in the DB (not just changed files) so
cross-file call targets resolve, deletes only edges sourced from
changed-file entities via `delete_structural_edges_by_sources`, and
re-inserts edges from changed files.

This is the same class of bug as the entity stale-marking issue
fixed during Phase 10 design review: a full-scan function called
with a partial file set causes data loss.
