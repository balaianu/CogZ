---
id: b2c3d4e5-6f7a-4b8c-9d0e-1f2a3b4c5d6e
title: Binary is the hook handler, no wrapper scripts
type: knowledge
category: decisions
tags: [hooks, architecture, agent-agnostic]
created_at: 2026-09-02T16:05:00Z
updated_at: 2026-09-02T16:05:00Z
status: active
---

CogZ's `capture-event` command is the hook handler itself. No wrapper
shell scripts are shipped or installed. The binary handles:

1. Repo discovery (cwd or `--repo`)
2. Silent skip when no `.cogz/` exists (`--hook-json` mode)
3. stdin parsing for agent hook payloads (`--hook-json` mode)
4. JSON output wrapping (`{"hookSpecificOutput": {...}}`)
5. FTS-only fast path (`--fts-only` skips ONNX model loading)

This keeps CogZ agent-agnostic. The docs show how to wire `cogz
capture-event` into any agent's hook config. No scripts to copy, no
paths to customize, no files to embed in the binary.

The `--fts-only` flag is critical for hook use: without it, each hook
call loads the ONNX runtime (~40s). With it, hook calls complete in
~0.5s using FTS-only search. The MCP server (persistent process) is
the preferred path when vector search is needed.
