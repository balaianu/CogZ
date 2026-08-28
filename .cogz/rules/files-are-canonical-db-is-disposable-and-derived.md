---
id: 78fe7be0-c777-4c7b-afab-1d378f16daeb
title: Files are canonical — DB is disposable and derived
type: rule
status: active
created_at: 2026-08-28T19:34:00Z
updated_at: 2026-08-28T19:34:00Z
references: []
confidence: 1.0
validation_count: 2
supporting_ids: []
---

Every write writes the file first, then syncs the DB. The DB is
disposable — `cogz reset` + `cogz index` must rebuild everything
from files alone.

**What this means in practice:**

- `update_knowledge` (Phase 7) overwrites the file, then re-syncs.
  It does not UPDATE the DB directly.
- Observations are append-only — no `update_observation` tool
  exists or will be added.
- Status changes edit frontmatter in-place, then sync. The status
  state machine (`transition_status`) validates transitions.
- Deleted files → DB entity marked `stale`, not deleted. Edges
  preserved.

**The rebuildability test:** After any change, `cogz reset` +
`cogz index` must produce the same DB state. If it doesn't, the
change broke the invariant.

**What code entities (Phase 8) break:** Code entities have no file
on disk — they're extracted from source by tree-sitter. They're
rebuildable from source code, not from `.cogz/` files. The
principle holds: the source of truth (source files) is canonical,
the DB is derived.
