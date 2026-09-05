---
id: e7fb0c1d-09cc-414e-831c-8f5710374d4a
title: File-save hook triggers reindex and stale flagging automatically
type: observation
status: stale
created_at: "2026-09-03T12:07:00Z"
updated_at: "2026-09-05T09:13:27.679308538+00:00"
references: []
source: agent
confidence: 0.95
supporting_ids: []
---

During the dogfooding run, the `file_save` hook was tested by
simulating a save on `src/update.rs`. The hook correctly:

1. Detected the file as a source file (not under `.cogz/`)
2. Triggered an incremental code reindex via git diff
3. Re-parsed the changed file with tree-sitter
4. Updated 27 code entities and created 8 new ones
5. Flagged 2 knowledge entries as stale (they reference code that
   changed)

The stale flagging works by checking if any knowledge/observation
entity's `references` point to code entities that were updated or
created during the reindex. When a referenced code entity changes,
the knowledge entry is marked stale via a file-first frontmatter
update (status → stale in the `.md` file, then synced to DB).

## What this means for agents

When an agent edits source code, the `file_save` hook automatically:
- Updates the code index (no manual `cogz reindex` needed)
- Flags knowledge that might be outdated
- The next `session_start` or `prompt_submit` hook will show stale
  knowledge in the context pack (with a stale marker), prompting
  the agent to verify or update it

This is the intended workflow: agents edit code, CogZ tracks what
knowledge might be affected, and future sessions are warned about
potentially outdated information.
