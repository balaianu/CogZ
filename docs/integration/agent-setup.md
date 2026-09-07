# Agent Setup

CogZ integrates with any AI coding agent that supports MCP servers or shell command hooks. This guide covers configuration for common agents.

## MCP server configuration

For any MCP-compatible agent, add CogZ as a server:

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"]
    }
  }
}
```

The server starts empty. Every tool call must include a `repo` parameter with the absolute path to the project root. There are no fallbacks — the agent must always state which repo it means.

See [MCP Tools](mcp-tools.md) for the full tool reference.

## Devin

### MCP server

Add to `~/.config/devin/mcp_config.json`:

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"]
    }
  }
}
```

### Hooks

Add to `~/.config/devin/hooks/hooks.v1.json`:

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

## Claude Code

### MCP server

Add to Claude Code's MCP configuration (`.claude/mcp.json` or via `claude mcp add`):

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"]
    }
  }
}
```

### Hooks

Claude Code supports hooks via `.claude/hooks.json`. The format is similar to Devin's:

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
    }]
  }
}
```

Adjust the hook event names and matchers to match Claude Code's supported events.

## Cursor

Cursor supports MCP servers via Settings > Features > MCP:

```json
{
  "mcpServers": {
    "cogz": {
      "command": "cogz",
      "args": ["mcp-stdio"]
    }
  }
}
```

Cursor does not currently support lifecycle hooks. Use the MCP tools directly — the agent can call `get_context` at the start of a task and `record_observation` when it learns something.

## Generic MCP client

Any MCP-compatible client can connect to CogZ. The server speaks MCP over stdio with protocol version negotiation handled by the `rmcp` SDK. No environment variables are required — the `repo` parameter on each tool call is the only configuration needed.

## Verifying the setup

After configuring, verify the MCP server is reachable:

```bash
# The server should start and wait for JSON-RPC on stdin
echo '{"jsonrpc":"2.0","method":"initialize","params":{},"id":1}' | cogz mcp-stdio
```

Verify hooks work:

```bash
cd ~/your-project
cogz capture-event session_start --hook-json --fts-only
```

This should print a JSON object with `hookSpecificOutput` containing the context pack, or `{}` if no `.cogz/` directory exists.

## Multiple repos

CogZ supports multiple repos from a single server instance. Each tool call specifies which repo it targets:

```json
{
  "tool": "search",
  "arguments": {
    "repo": "/home/user/project-a",
    "query": "auth flow"
  }
}
```

```json
{
  "tool": "search",
  "arguments": {
    "repo": "/home/user/project-b",
    "query": "auth flow"
  }
}
```

The server caches repo states and shares model instances across repos that use the same models.
