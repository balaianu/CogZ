---
id: 35cfc0d9-a3e6-4210-9e67-89ee82ae85aa
title: update_knowledge file path resolution
type: observation
status: stale
created_at: "2026-08-29T02:45:00Z"
updated_at: "2026-09-06T07:32:43.323531547+00:00"
references: []
source: agent
---

The `file_path` stored in the DB for entities synced from `.cogz/` files
is relative to the cogz_dir, not prefixed with `.cogz/`. For example, a
knowledge entry in category "test" with title "Foo" has
`file_path = "knowledge/test/foo.md"` in the DB, not
`.cogz/knowledge/test/foo.md`.

The `update_knowledge` helper initially checked `if file_path.starts_with(".cogz")`
and joined with `cogz_dir.parent()` for that case, falling through to
`PathBuf::from(file_path)` for the else case. But the else case produced
a relative path that doesn't exist from the process's CWD.

Fix: join relative paths with `cogz_dir` directly. The `.cogz` prefix
case is kept for backwards compatibility but shouldn't occur with the
current sync code.
