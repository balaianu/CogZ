---
id: c5d9eafb-09cc-414e-831c-8f5710374d4a
title: Audit found 11 issues across 7 categories
type: observation
status: stale
created_at: "2026-09-03T12:05:00Z"
updated_at: "2026-09-04T09:12:03.632169291+00:00"
references: []
source: agent
confidence: 0.95
supporting_ids: []
---

The second full code audit (2026-09-03) found 11 confirmed issues
after validation. All were fixed and tested.

## Categories and counts

- **File sync (3):** sync_single_file missing reference sync,
  incremental sync not restoring forward references, MCP writes
  doing full directory scans
- **Consolidation (2):** merge_one transaction leak on error,
  dedup matching against non-active entities
- **Doctor (1):** false-positive MissingFile for stale entities
- **Update (2):** checksum verification silently skipped on
  missing entry, predictable temp file names
- **Embed runtime (1):** ONNX Runtime download without checksum
- **Prune (1):** file deletion before DB tombstone
- **Background embed (1):** temp ID file leak on child crash

## Patterns observed

Three issues were ordering bugs (doing the right operations in the
wrong order): prune deleted before tombstoning, merge used `?`
without rollback, forward references only synced for changed files.

Two issues were missing filters: dedup didn't filter by status,
doctor didn't exclude stale from missing-file check.

Two issues were security-related: checksum skip and predictable
temp files.

## What worked well

The file-first invariant prevented data loss in most cases — even
when the DB had bugs, the canonical files were intact. The
rebuildability test (`cogz reset` + `cogz index`) was the key
validation tool.
