# Agent Setup

CogZ integrates with any AI coding agent that supports MCP servers or shell command hooks. This guide covers configuration for the 6 supported agents.

**Fast path:** `cogz init --configure auto` detects installed agents and writes both the MCP server entry and lifecycle hooks for all of them — project-scoped files by default, `--global` for user-level config. `cogz configure <harness>` does the same for already-initialized repos. Both merge into existing files (with `.cogz.bak` backups) and never clobber other tools' config. The rest of this page documents what gets written and the manual alternative.

All 6 agents support both MCP and hooks, and all support global (user-level) and project-scoped config for both. The MCP server entry is near-identical across agents — file location and a few required fields differ (Cursor needs `"type": "stdio"`, Copilot needs `"type": "local"` + a `tools` allowlist, Codex uses TOML). Hook config varies by agent; see [Hooks](hooks.md) for the full event reference and canonical hook JSON.

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

`command` should be the absolute path to the installed binary (e.g. `/home/you/.local/bin/cogz`) — `cogz configure` resolves and writes it automatically. A bare `cogz` only resolves when the client's spawn environment has the install directory on `PATH`; agents launched by daemons or desktop apps frequently do not.

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

Hook event names are largely standardized — Claude Code's naming is the de facto standard that Codex and Devin also speak — but file formats and a few event names diverge per agent (see the per-agent sections below). `cogz configure` emits each agent's native shape.

## Effect coverage per agent

What each agent's hook surface can actually deliver, verified against official docs. "Inject" = hook stdout context reaches the model (`additionalContext`/`additional_context`).

| Effect | Claude Code | Cursor | Codex | Gemini CLI | Copilot CLI | Devin |
|---|---|---|---|---|---|---|
| Session pack on start | inject | inject (`additional_context`, fire-and-forget) | inject | inject | inject (`sessionStart` only field consumed) | inject |
| Task pack per prompt | inject (`UserPromptSubmit`) | no channel (`beforeSubmitPrompt` returns `continue` only → record-only) | inject | inject (`BeforeAgent`) | output dropped (`userPromptSubmitted` → record-only) | inject |
| Tool-result nudges | inject (`PostToolUse`) | inject (`postToolUse`) | inject | inject (`AfterTool`) | inject (`postToolUse`) | inject |
| File-save reindex | `PostToolUse` matcher | `afterFileEdit` (native) | `PostToolUse` matcher | `AfterTool` matcher | none (no post-edit event) | `PostToolUse` matcher |
| Post-compaction re-injection | via `SessionStart` refire on compact (`PostCompact` itself cannot inject) | none (`preCompact` is observe-only; no post event) | via `SessionStart` compact source (`PostCompact` cannot inject) | none (`PreCompress` is advisory-only; no post event) | none (no event) | inject (`PostCompaction`) |
| Session-end consolidation | fires | fires (fire-and-forget) | fires — **3s cap**, consolidation best-effort | advisory | fires | fires |

Compaction hooks on non-injecting surfaces run with `--record-only` — the event is captured and the debounced reindex fires, but stdout stays empty instead of emitting a pack the consumer would reject or ignore.

## Per-agent setup

### Claude Code

**MCP config (project):** `.mcp.json` in the project root, or via `claude mcp add cogz cogz mcp-stdio`

**MCP config (global):** `~/.claude.json` under the top-level `mcpServers` key, or via `claude mcp add cogz cogz mcp-stdio --scope user`

**Hooks config (project):** `.claude/settings.json` under the `hooks` key

**Hooks config (global):** `~/.claude/settings.json` under the `hooks` key

Claude Code supports hooks via the `hooks` key in its settings JSON, using matcher groups `{matcher?, hooks: [{type: "command", command, timeout?}]}` (timeout in seconds) — the [canonical hook config](hooks.md#full-hook-configuration). CogZ uses `SessionStart`, `UserPromptSubmit`, `PostToolUse` (incl. an `Edit|Write|MultiEdit|NotebookEdit` matcher for `file_save`), `SessionEnd`, `Stop`, and `PostCompact`.

`PostCompact` itself cannot inject context (its output schema has no `hookSpecificOutput` — stdout goes to the debug log), but `SessionStart` refires after compaction (`source: "compact"`), so post-compaction re-injection lands through the session-start hook regardless; the `PostCompact` entry runs `--record-only` — the compaction event and the debounced reindex still fire, with empty stdout.

### Cursor

**MCP config (project):** `.cursor/mcp.json` in the project root

**MCP config (global):** `~/.cursor/mcp.json`

**Hooks config (project):** `.cursor/hooks.json`

**Hooks config (global):** `~/.cursor/hooks.json`

Cursor supports both MCP servers and lifecycle hooks. `hooks.json` requires a top-level `"version": 1` and uses **flat** hook entries — `{"command": "...", "matcher"?, "timeout"?}` — not Claude's nested matcher-group shape. Event names are camelCase and differ from Claude's: `sessionStart`, `beforeSubmitPrompt`, `postToolUse`, `afterFileEdit`, `sessionEnd`, `stop`, `preCompact`. CogZ maps `file_save` onto the native `afterFileEdit` event (no tool matcher needed) and records the compaction on `preCompact` — observe-only with no post event, so it runs `--record-only` and nothing re-injects.

Cursor's MCP entries require an explicit `"type": "stdio"` field — a missing or Copilot-style `local` type can fail registration silently, so `configure` emits `{"type": "stdio", "command": "/path/to/cogz", "args": ["mcp-stdio"]}` (the resolved absolute path). Its context-injection stdout field is `additional_context` (snake_case), so `cogz configure` emits commands with `--hook-format cursor`. `beforeSubmitPrompt`'s output schema is only `continue`/`user_message` — it cannot inject context — so the prompt hook runs `--fts-only` (event still recorded). Cursor also auto-maps Claude Code hook names when loading Claude-format configs — see [Third Party Hooks](https://cursor.com/docs/reference/third-party-hooks.md).

### Codex (OpenAI)

**MCP config (project):** `.codex/config.toml` under `[mcp_servers.cogz]` (trusted projects only)

**MCP config (global):** `~/.codex/config.toml` under `[mcp_servers.cogz]`

**Hooks config (project):** `.codex/hooks.json`

**Hooks config (global):** `~/.codex/hooks.json`

Codex MCP uses TOML, not JSON. The MCP server entry looks like:

```toml
[mcp_servers.cogz]
command = "/path/to/cogz" # absolute path to the installed binary
args = ["mcp-stdio"]
```

Codex hooks use the same JSON format and PascalCase event names as Claude Code (including `PostCompact`). Hooks require explicit trust review before running — use `/hooks` in the CLI to review and trust CogZ hooks after adding them. Codex also merges `hooks.json` with inline `[hooks]` tables in `config.toml` when both exist (with a startup warning); CogZ writes `hooks.json` only.

Codex-specific limits: `SessionEnd` (and `Interrupt`) hooks are capped at **3 seconds** (1s default) — `configure` writes `timeout: 3`, so consolidation runs best-effort inside the cap. `PostCompact` cannot inject context (strict per-event output schema with no context field — plain stdout is ignored as debug output), so it runs `--record-only`; `SessionStart` accepts a `compact` matcher source, and post-compaction re-injection arrives through the session-start hook. Non-managed hooks are hash-pinned: any edit to a hook command marks it for re-review before it runs again.

### Gemini CLI (Google)

**MCP config (project):** `.gemini/settings.json` under the `mcpServers` key

**MCP config (global):** `~/.gemini/settings.json` under the `mcpServers` key

**Hooks config (project):** `.gemini/settings.json` under the `hooks` key (same file as MCP)

**Hooks config (global):** `~/.gemini/settings.json` under the `hooks` key (same file as MCP)

Gemini CLI stores both MCP and hooks config in the same `settings.json` file. The hook shape mirrors Claude's matcher groups (`{matcher?, hooks: [{type: "command", command, name?, timeout?}]}`), but event names differ — `SessionStart`, `BeforeAgent` (user prompt), `AfterTool`, `SessionEnd`, `AfterAgent` (turn end), `PreCompress` — and **`timeout` is in milliseconds**, not seconds (default 60000). `AfterTool` matchers filter on tool names, so `file_save` uses `write_file|replace|edit|notebook_edit`. `BeforeAgent`, `AfterTool`, and `SessionStart` all honor `hookSpecificOutput.additionalContext` — packs and nudges inject normally. Gemini types `hookSpecificOutput.hookEventName` per event (a `BeforeAgent` firing requires exactly `"BeforeAgent"`), and sends `hook_event_name` on stdin — CogZ echoes it back so the tag always matches the firing event. `PreCompress` is advisory-only (`systemMessage` output only, fires *before* compression) and Gemini has no post-compress event, so it runs `--record-only` and compaction re-injection isn't possible there. See the [Gemini CLI hooks reference](https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/reference.md) for the full event list.

### GitHub Copilot CLI

**MCP config (project):** `.mcp.json` or `.github/mcp.json` in the project root

**MCP config (global):** `~/.copilot/mcp-config.json`

**Hooks config (project):** `.github/hooks/cogz.json` (one file per hook set in the hooks directory)

**Hooks config (global):** `~/.copilot/hooks/cogz.json`

Copilot's MCP entries differ from the common shape — `configure` emits `{"type": "local", "command": "/path/to/cogz", "args": ["mcp-stdio"], "tools": ["*"]}` (the resolved absolute path). `type` is `local` (not `stdio`), and without a `tools` allowlist the server's tools are not exposed.

Copilot CLI loads hooks from JSON files in the hooks directory — each file is a separate hook set. The format uses a `version` field (`1`) and a `hooks` object with flat entries — `{"type": "command", "command"|"bash"|"powershell"|"exec", "cwd"?, "env"?, "timeoutSec"?}`; `command` is the cross-platform fallback Copilot copies to both `bash` and `powershell`.

`cogz configure` writes PascalCase event names (`SessionStart`, `UserPromptSubmit`, `PostToolUse`, `SessionEnd`, `Stop`) — under PascalCase, Copilot delivers snake_case stdin payloads (`tool_name`, `tool_input`, `tool_result`), matching CogZ's hook parser. Its context-injection stdout field is top-level `additionalContext`, so commands pass `--hook-format copilot`. Two coverage limits by design: `UserPromptSubmit` hook output is dropped by Copilot's command-hook runtime (prompt_submit runs `--fts-only`, record-only), and the versioned format has no matcher field, so `file_save` has no Copilot analog. See the [Copilot hooks reference](https://docs.github.com/en/copilot/reference/hooks-reference) for the full event list.

### Devin

**MCP config (project):** `.devin/mcp_config.json` (committed) or `.devin/mcp_config.local.json` (gitignored)

**MCP config (global):** `~/.config/devin/mcp_config.json`

**Hooks config (project):** `.devin/hooks.v1.json` (standalone file, recommended) or `.devin/config.json` under the `hooks` key

**Hooks config (global):** `~/.config/devin/config.json` under the `hooks` key

Devin supports the full hook lifecycle: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `FileSave` (via `PostToolUse` matcher on `edit|write|notebook_edit`), `SessionEnd`, `Stop`, and `PostCompaction`.

See [Hooks](hooks.md#full-hook-configuration) for the canonical JSON config. Devin-specific notes:

- `.devin/hooks.v1.json` *is* the hooks object — the event map sits at the top level with no `hooks` wrapper key (`configure` writes it that way). The `config.json` variants do nest the same map under `hooks`.
- `PostCompaction` re-injects context after compaction by calling `session_start`. Devin sends `hook_event_name` on stdin and CogZ echoes it, so the emitted `hookSpecificOutput.hookEventName` carries `PostCompaction` — the name the firing event's output schema expects.
- `FileSave` is implemented as a `PostToolUse` hook with matcher `edit|write|notebook_edit` rather than a separate event type.

### Generic MCP client

Any MCP-compatible client can connect to CogZ. The server speaks MCP over stdio with protocol version negotiation handled by the `rmcp` SDK. No environment variables are required — the `repo` parameter on each tool call is the only configuration needed.

For agents without hook support, the agent can call `get_context` (cold_start mode) at the start of a session and `create_entity` (observation) when it learns something. This gives most of the benefit of hooks without lifecycle integration.

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

This should print a JSON object with `hookSpecificOutput` containing the context pack, or print nothing if no `.cogz/` directory exists.

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
