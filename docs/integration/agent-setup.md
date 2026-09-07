# Agent Setup

CogZ integrates with any AI coding agent that supports MCP servers or shell command hooks. This guide covers configuration for common agents.

The MCP server configuration is the same for all agents — only the config file location differs. Hook configuration varies by agent; see [Hooks](hooks.md) for the full event reference and canonical hook JSON config.

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

The server starts empty. Every tool call must include a `repo` parameter with the absolute path to the project root. Per the 2026-07-28 MCP spec, there are no Roots and no session state — the agent must always state which repo it means.

See [MCP Tools](mcp-tools.md) for the full tool reference.

## Per-agent setup

### Devin

**MCP config:** `~/.config/devin/mcp_config.json`

**Hooks config:** `~/.config/devin/hooks/hooks.v1.json`

Devin supports the full hook lifecycle: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `FileSave` (via `PostToolUse` matcher on `edit|write|notebook_edit`), `SessionEnd`, `Stop`, and `PostCompaction`.

See [Hooks](hooks.md#full-hook-configuration) for the canonical JSON config. Devin-specific notes:

- `PostCompaction` re-injects context after compaction by calling `session_start` — add it if you want context preserved across compaction events.
- `FileSave` is implemented as a `PostToolUse` hook with matcher `edit|write|notebook_edit` rather than a separate event type.

### Claude Code

**MCP config:** `.claude/mcp.json` in the project root, or via `claude mcp add cogz cogz mcp-stdio`

**Hooks config:** `.claude/hooks.json` in the project root

Claude Code supports hooks via `.claude/hooks.json`. The event names and format match the [canonical hook config](hooks.md#full-hook-configuration). Supported events: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`.

Claude Code does not currently support `SessionEnd` or `PostCompaction` hooks. To run consolidation, use `cogz consolidate` manually or via the MCP `consolidate` tool.

### Cursor

**MCP config:** Settings > Features > MCP, or `.cursor/mcp.json` in the project root

**Hooks:** Not supported.

Cursor supports MCP servers but not lifecycle hooks. Use the MCP tools directly — the agent can call `get_context` at the start of a task and `record_observation` when it learns something.

### Codex (OpenAI)

**MCP config:** Add to Codex's MCP server configuration (see [Codex docs](https://developers.openai.com/codex/))

**Hooks:** Not supported.

Codex supports MCP servers. Use the MCP tools directly for context retrieval and observation recording.

### Windsurf

**MCP config:** Windsurf Settings > MCP Servers

**Hooks:** Not supported.

Windsurf supports MCP servers. Use the MCP tools directly for context retrieval and observation recording.

### Generic MCP client

Any MCP-compatible client can connect to CogZ. The server speaks MCP over stdio with protocol version negotiation handled by the `rmcp` SDK. No environment variables are required — the `repo` parameter on each tool call is the only configuration needed.

For agents without hook support, the agent can call `get_context` (cold_start mode) at the start of a session and `record_observation` when it learns something. This gives most of the benefit of hooks without lifecycle integration.

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
