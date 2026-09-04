---
id: d0e4f5a6-09cc-414e-831c-8f5710374d4a
title: Prune must tombstone DB before deleting canonical file
type: knowledge
status: stale
created_at: "2026-09-03T11:58:00Z"
updated_at: "2026-09-04T09:20:00.899401395+00:00"
references: []
category: gotchas
tags: ["prune", "doctor", "tombstone", "file-first", "ordering"]
---

# Prune must tombstone DB before deleting canonical file

The file-first invariant says "write the file first, then sync the
DB." For deletion (prune), the inverse applies: **tombstone the DB
first, then delete the file.**

If the order is reversed (delete file, then tombstone DB), a failure
in `tombstone_entity` leaves the system in an unrecoverable state:
the canonical file is gone, but the DB entity is still active. The
content is lost permanently — `cogz reset` + `cogz index` cannot
rebuild from a deleted file.

## The correct order

1. `tombstone_entity(&conn, &entity_id)` — DB entity becomes a
   tombstone (status = "pruned", content/embedding/FTS removed)
2. `delete_embedding(&conn, &entity_id)` — remove vec0 entry
3. `std::fs::remove_file(&file_path)` — delete the canonical file

If step 1 fails, the file is still on disk and the entity is still
in its terminal state (rejected/superseded). This is safe — the
prune can be retried. If step 3 fails, the entity is already
tombstoned — the orphaned file is cosmetic and will be cleaned up
on the next sync.

## Where this was found

`run_prune` in `src/doctor/prune.rs` deleted the file first, then
called `tombstone_entity`. Found in the second full code audit
(2026-09-03).
