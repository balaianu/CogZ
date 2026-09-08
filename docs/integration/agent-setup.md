# Agent Setup

CogZ integrates with any AI coding agent that supports MCP servers or shell command hooks. This guide covers configuration for the 6 supported agents.

All 6 agents support both MCP and hooks, and all support global (user-level) and project-scoped config for both. The MCP server config is the same for all agents — only the file location and format differ. Hook config varies by agent; see [Hooks](hooks.md) for the full event reference and canonical hook JSON.

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

## Support matrix

| Agent | Project MCP | Global MCP | Project hooks | Global hooks | Format |
|---|---|---|---|---|---|
| Claude Code | `.mcp.json` | `~/.claude.json` | `.claude/settings.json` | `~/.claude/settings.json` | JSON |
| Cursor | `.cursor/mcp.json` | `~/.cursor/mcp.json` | `.cursor/hooks.json` | `~/.cursor/hooks.json` | JSON |
| Codex | `.codex/config.toml` | `~/.codex/config.toml` | `.codex/hooks.json` | `~/.codex/hooks.json` | TOML (MCP) / JSON (hooks) |
| Gemini CLI | `.gemini/settings.json` | `~/.gemini/settings.json` | `.gemini/settings.json` | `~/.gemini/settings.json` | JSON |
| Copilot CLI | `.mcp.json` | `~/.copilot/mcp-config.json` | `.github/hooks/*.json` | `~/.copilot/hooks/*.json` | JSON |
| Devin | `.devin/mcp_config.json` | `~/.config/devin/mcp_config.json` | `.devin/hooks.v1.json` | `~/.config/devin/config.json` (`hooks` key) | JSON |

Hook event names are largely standardized — Claude Code's naming is the de facto standard. Cursor auto-maps Claude Code hook names. Codex reuses the same lifecycle event names. The canonical hook config from [Hooks](hooks.md) works across all agents with minor path adjustments.

## Per-agent setup

### Claude Code

**MCP config (project):** `.mcp.json` in the project root, or via `claude mcp add cogz cogz mcp-stdio`

**MCP config (global):** `~/.claude.json` under the top-level `mcpServers` key, or via `claude mcp add cogz cogz mcp-stdio --scope user`

**Hooks config (project):** `.claude/settings.json` under the `hooks` key

**Hooks config (global):** `~/.claude/settings.json` under the `hooks` key

Claude Code supports hooks via the `hooks` key in its settings JSON. The event names and format match the [canonical hook config](hooks.md#full-hook-configuration). Supported events: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`.

Claude Code does not currently support `SessionEnd` or `PostCompaction` hooks. To run consolidation, use `cogz consolidate` manually or via the MCP `consolidate` tool.

### Cursor

**MCP config (project):** `.cursor/mcp.json` in the project root

**MCP config (global):** `~/.cursor/mcp.json`

**Hooks config (project):** `.cursor/hooks.json`

**Hooks config (global):** `~/.cursor/hooks.json`

Cursor supports both MCP servers and lifecycle hooks. Hook event names use camelCase (`sessionStart`, `preToolUse`, etc.) and auto-map from Claude Code's PascalCase names. Cursor also loads Claude Code's `.claude/settings.json` hooks directly if third-party hook support is enabled in Settings.

### Codex (OpenAI)

**MCP config (project):** `.codex/config.toml` under `[mcp_servers.cogz]` (trusted projects only)

**MCP config (global):** `~/.codex/config.toml` under `[mcp_servers.cogz]`

**Hooks config (project):** `.codex/hooks.json`

**Hooks config (global):** `~/.codex/hooks.json`

Codex MCP uses TOML, not JSON. The MCP server entry looks like:

```toml
[mcp_servers.cogz]
command = "cogz"
args = ["mcp-stdio"]
```

Codex hooks use the same JSON format and event names as Claude Code. Hooks require explicit trust review before running — use `/hooks` in the CLI to review and trust CogZ hooks after adding them.

### Gemini CLI (Google)

**MCP config (project):** `.gemini/settings.json` under the `mcpServers` key

**MCP config (global):** `~/.gemini/settings.json` under the `mcpServers` key

**Hooks config (project):** `.gemini/settings.json` under the `hooks` key (same file as MCP)

**Hooks config (global):** `~/.gemini/settings.json` under the `hooks` key (same file as MCP)

Gemini CLI stores both MCP and hooks config in the same `settings.json` file. Hook event names use PascalCase (`SessionStart`, `BeforeTool`, `AfterTool`, etc.) and follow a similar pattern to Claude Code. See the [Gemini CLI hooks reference](https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/reference.md) for the full event list.

### GitHub Copilot CLI

**MCP config (project):** `.mcp.json` or `.github/mcp.json` in the project root

**MCP config (global):** `~/.copilot/mcp-config.json`

**Hooks config (project):** `.github/hooks/cogz.json` (one file per hook set in the hooks directory)

**Hooks config (global):** `~/.copilot/hooks/cogz.json`

Copilot CLI loads hooks from JSON files in the hooks directory — each file is a separate hook set. The format uses a `version` field and a `hooks` object with event names in camelCase (`sessionStart`, `preToolUse`, `postToolUse`, etc.). See the [Copilot hooks reference](https://docs.github.com/en/copilot/reference/hooks-reference) for the full event list.

### Devin

**MCP config (project):** `.devin/mcp_config.json` (committed) or `.devin/mcp_config.local.json` (gitignored)

**MCP config (global):** `~/.config/devin/mcp_config.json`

**Hooks config (project):** `.devin/hooks.v1.json` (standalone file, recommended) or `.devin/config.json` under the `hooks` key

**Hooks config (global):** `~/.config/devin/config.json` under the `hooks` key

Devin supports the full hook lifecycle: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `FileSave` (via `PostToolUse` matcher on `edit|write|notebook_edit`), `SessionEnd`, `Stop`, and `PostCompaction`.

See [Hooks](hooks.md#full-hook-configuration) for the canonical JSON config. Devin-specific notes:

- `PostCompaction` re-injects context after compaction by calling `session_start` — add it if you want context preserved across compaction events.
- `FileSave` is implemented as a `PostToolUse` hook with matcher `edit|write|notebook_edit` rather than a separate event type.

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
