# Hooks

Hooks capture lifecycle events from an AI coding agent and inject context packs into the agent's session. CogZ's binary is the hook handler — no wrapper scripts needed.

## How it works

When an agent fires a lifecycle event (session start, prompt submit, tool use, file save, etc.), it calls `cogz capture-event` with the event type and optional context. CogZ:

1. Records the event in the database (audit trail).
2. For `session_start` and `prompt_submit`: assembles a context pack and prints it to stdout for the agent to consume as injected context.
3. For `file_save`: triggers an incremental code reindex and flags stale knowledge if the saved file is a source file.
4. For `session_end`: runs consolidation (promotion + merge).

## The `--hook-json` flag

When `--hook-json` is passed, output is wrapped in the JSON format expected by agent hook systems:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "session_start",
    "additionalContext": "...context pack as text..."
  }
}
```

If no `.cogz/` directory is found in the current repo, CogZ prints `{}` and exits silently — the hook is a no-op for repos that don't use CogZ.

## The `--fts-only` flag

The `--fts-only` flag skips ONNX model loading and uses FTS-only search for context assembly. This is critical for hook use:

- **Without `--fts-only`:** each hook call loads the ONNX runtime (~40s on first load, ~2s on subsequent calls with warm models).
- **With `--fts-only`:** hook calls complete in ~0.5s using FTS-only search.

For hooks, speed matters more than ranking quality. The MCP server (persistent process) is the preferred path when vector search is needed — it keeps models loaded across calls.

## Event types

| Event | When | What CogZ does | Output |
|---|---|---|---|
| `session_start` | Agent session begins | Records event, assembles cold_start context pack | Context pack (recent rules + observations) |
| `prompt_submit` | User submits a prompt | Records event, assembles task context pack using the prompt | Context pack (query-scoped retrieval) |
| `pre_tool_use` | Before a tool call | Records event only | None (audit trail) |
| `post_tool_use` | After a tool call | Records event only | None (audit trail) |
| `file_save` | A file is saved | Records event, triggers incremental code reindex if source file, flags stale knowledge | Reindex summary |
| `session_end` | Agent session ends | Records event, runs consolidation (promotion + merge) | Consolidation summary |
| `stop` | Agent stops | Records event only | None |

## CLI usage

```bash
# Session start (from a hook config)
cogz capture-event session_start --hook-json --fts-only

# Prompt submit (from a hook config, reading prompt from stdin or file)
cogz capture-event prompt_submit --hook-json --fts-only --prompt "implement auth"

# File save (from a hook config, triggered on edit/write)
cogz capture-event file_save --hook-json --fts-only --file-path src/auth.rs

# Session end (from a hook config)
cogz capture-event session_end --hook-json --fts-only

# Manual (no hook JSON, plain text output)
cogz capture-event session_start --repo ~/my-project
```

## Full hook configuration

This is the canonical hook config. Copy it into your agent's hook configuration file — see [Agent Setup](agent-setup.md) for the file path and any agent-specific differences (event names, matchers, supported events).

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "cogz capture-event session_start --hook-json --fts-only",
        "timeout": 10
      }]
    }],
    "UserPromptSubmit": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "cogz capture-event prompt_submit --hook-json --fts-only",
        "timeout": 10
      }]
    }],
    "PostToolUse": [
      {
        "matcher": "",
        "hooks": [{
          "type": "command",
          "command": "cogz capture-event post_tool_use --hook-json --fts-only",
          "timeout": 10
        }]
      },
      {
        "matcher": "edit|write|notebook_edit",
        "hooks": [{
          "type": "command",
          "command": "cogz capture-event file_save --hook-json --fts-only",
          "timeout": 20
        }]
      }
    ],
    "SessionEnd": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "cogz capture-event session_end --hook-json --fts-only",
        "timeout": 30
      }]
    }],
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "cogz capture-event stop --hook-json --fts-only",
        "timeout": 5
      }]
    }]
  }
}
```

**Timeouts:** `session_start` and `prompt_submit` need 10s (FTS-only context assembly). `file_save` needs 20s (incremental reindex). `session_end` needs 30s (consolidation). `stop` needs 5s (event recording only).

**PostCompaction:** After context compaction, the agent loses its injected context. Re-inject by treating it as a session start:

```json
"PostCompaction": [{
  "matcher": "",
  "hooks": [{
    "type": "command",
    "command": "cogz capture-event session_start --hook-json --fts-only",
    "timeout": 10
  }]
}]
```

## Context pack output

For `session_start` and `prompt_submit`, the context pack is printed as formatted text inside the `additionalContext` field of the hook JSON output. The pack includes:

- **Sections** — each with an entity's title, type, and content (possibly truncated to fit the token budget)
- **Source priority** — rules > observations > knowledge > code
- **Graph provenance** — for task/escalation mode, each section includes the graph path from the query match to this entity
- **Token budget** — sections are sorted by priority then relevance, and truncated/dropped to fit the configured budget

## See also

- [Agent Setup](agent-setup.md) — per-agent config file locations and supported events
- [CLI Reference](../cli-reference.md) — all `capture-event` flags
- [Design: Degradation](../design/degradation.md) — how hooks work without models
