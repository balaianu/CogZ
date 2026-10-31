//! Harness table and per-agent config payloads. Event names, file
//! locations, and entry shapes are verified against each agent's
//! official hook/MCP docs; divergences are recorded in the
//! `agent-config-format-divergences` knowledge entry and
//! docs/integration/agent-setup.md.

use std::path::{Path, PathBuf};

use super::{
    hook_payloads::{claude_shaped_patch, hooks_codex, hooks_devin},
    shell_argv0,
};
use serde_json::{Value, json};

/// A single file to create or merge into.
pub(super) enum Patch {
    /// Deep-merge a JSON object into the file (parsed or `{}`).
    Json(PathBuf, Value),
    /// Merge a TOML table into the file (parsed or empty table).
    Toml(PathBuf, toml::Value),
}

/// One supported agent: identity, detection markers, file plan.
pub(super) struct Harness {
    pub id: &'static str,
    /// Marker paths relative to the repo root.
    repo_markers: &'static [&'static str],
    /// Marker paths relative to the user's home.
    home_markers: &'static [&'static str],
    pub project_patches: fn(&Path, &str) -> Vec<Patch>,
    pub user_patches: fn(&Path, &str) -> Vec<Patch>,
}

// ─── Payloads ─────────────────────────────────────────────────────

fn mcp_entry(bin: &str) -> Value {
    json!({"command": bin, "args": ["mcp-stdio"]})
}

/// Cursor's hooks.json is `{"version": 1, "hooks": {camelCase:
/// [{"command", "matcher"?, "timeout"?}]}}` — flat entries, not the
/// Claude matcher-group shape. `afterFileEdit` is a native event so
/// file_save needs no tool-name matcher; `preCompact` covers
/// compaction. Cursor's stdout contract is `additional_context`
/// (snake_case), so generated commands pass `--hook-format cursor`.
/// `beforeSubmitPrompt` cannot inject context (output schema is
/// continue/user_message only), so prompt_submit runs --fts-only:
/// the event is recorded without generating an unconsumable pack.
/// `preCompact` is observe-only the same way — the compaction event
/// is recorded, nothing is emitted.
fn cursor_hooks_patch(path: PathBuf, bin: &str) -> Patch {
    let sb = shell_argv0(bin);
    let e = |args: &str, timeout: u64| -> Value {
        json!([{
            "command": format!("{sb} capture-event {args} --hook-json --hook-format cursor"),
            "timeout": timeout
        }])
    };
    Patch::Json(
        path,
        json!({
            "version": 1,
            "hooks": {
                "sessionStart": e("session_start", 15),
                "beforeSubmitPrompt": e("prompt_submit --fts-only", 15),
                "postToolUse": e("post_tool_use --fts-only", 10),
                "afterFileEdit": e("file_save --fts-only", 20),
                "sessionEnd": e("session_end --fts-only", 30),
                "stop": e("stop --fts-only", 5),
                "preCompact": e("session_start --fts-only --record-only", 15),
            }
        }),
    )
}

/// Copilot hook sets are standalone versioned JSON files under
/// .github/hooks/ (project) or ~/.copilot/hooks/ (user). Entries are
/// flat: `command` is the cross-platform fallback Copilot copies to
/// bash and powershell; `timeoutSec` is seconds. PascalCase event
/// names switch stdin payloads to snake_case (matching the fields
/// `cogz capture-event` parses). Copilot's context-injection field is
/// top-level `additionalContext` → `--hook-format copilot`.
/// `UserPromptSubmit` hook output is dropped by Copilot's command-hook
/// runtime, so prompt_submit runs --fts-only (record-only). Copilot's
/// versioned format has no matcher field, so file_save has no analog.
fn copilot_hooks_patch(path: PathBuf, bin: &str) -> Patch {
    let sb = shell_argv0(bin);
    let e = |args: &str, timeout: u64| -> Value {
        json!([{
            "type": "command",
            "command": format!("{sb} capture-event {args} --hook-json --hook-format copilot"),
            "timeoutSec": timeout
        }])
    };
    Patch::Json(
        path,
        json!({
            "version": 1,
            "hooks": {
                "SessionStart": e("session_start", 15),
                "UserPromptSubmit": e("prompt_submit --fts-only", 15),
                "PostToolUse": e("post_tool_use --fts-only", 10),
                "SessionEnd": e("session_end --fts-only", 30),
                "Stop": e("stop --fts-only", 5),
            }
        }),
    )
}

/// Gemini nests hooks in settings.json beside mcpServers. Shape is
/// Claude-like matcher groups, but event names differ (BeforeAgent,
/// AfterTool, AfterAgent, PreCompress) and `timeout` is in
/// MILLISECONDS (default 60000), not seconds. Gemini sends
/// `hook_event_name` on stdin and requires that literal in
/// hookSpecificOutput — capture-event echoes it back, so the native
/// names tag correctly. PreCompress is advisory-only (no context
/// injection) — record-only keeps the event record silent.
fn gemini_settings_patch(path: PathBuf, bin: &str) -> Patch {
    let sb = shell_argv0(bin);
    let g = |args: &str, timeout_ms: u64| -> Value {
        json!([{
            "name": "cogz",
            "type": "command",
            "command": format!("{sb} capture-event {args} --hook-json"),
            "timeout": timeout_ms
        }])
    };
    Patch::Json(
        path,
        json!({
            "mcpServers": {"cogz": mcp_entry(bin)},
            "hooks": {
                "SessionStart": [{"hooks": g("session_start", 15000)}],
                "BeforeAgent": [{"matcher": "*", "hooks": g("prompt_submit", 15000)}],
                "AfterTool": [
                    {"hooks": g("post_tool_use --fts-only", 10000)},
                    {
                        "matcher": "write_file|replace|edit|notebook_edit",
                        "hooks": g("file_save --fts-only", 20000)
                    },
                ],
                "SessionEnd": [{"hooks": g("session_end --fts-only", 30000)}],
                "AfterAgent": [{"matcher": "*", "hooks": g("stop --fts-only", 5000)}],
                "PreCompress": [{"hooks": g("session_start --fts-only --record-only", 15000)}],
            }
        }),
    )
}

fn mcp_json_patch(path: PathBuf, bin: &str) -> Patch {
    mcp_json_patch_with(path, mcp_entry(bin))
}

/// Cursor requires an explicit `type` on stdio servers — a missing or
/// Copilot-style `local` type fails registration silently.
fn cursor_mcp_patch(path: PathBuf, bin: &str) -> Patch {
    mcp_json_patch_with(
        path,
        json!({"type": "stdio", "command": bin, "args": ["mcp-stdio"]}),
    )
}

/// Copilot's MCP shape is `type: "local"` plus a `tools` allowlist —
/// without `tools` the server's tools are not exposed.
fn copilot_mcp_patch(path: PathBuf, bin: &str) -> Patch {
    mcp_json_patch_with(
        path,
        json!({"type": "local", "command": bin, "args": ["mcp-stdio"], "tools": ["*"]}),
    )
}

fn mcp_json_patch_with(path: PathBuf, entry: Value) -> Patch {
    Patch::Json(path, json!({"mcpServers": {"cogz": entry}}))
}

fn codex_mcp_patch(path: PathBuf, bin: &str) -> Patch {
    let mut cogz_server = toml::Table::new();
    cogz_server.insert("command".into(), bin.into());
    cogz_server.insert("args".into(), vec!["mcp-stdio"].into());
    let mut mcp_servers = toml::Table::new();
    mcp_servers.insert("cogz".into(), cogz_server.into());
    let mut root = toml::Table::new();
    root.insert("mcp_servers".into(), mcp_servers.into());
    Patch::Toml(path, root.into())
}

// ─── Harness table ────────────────────────────────────────────────

fn claude_project(repo: &Path, bin: &str) -> Vec<Patch> {
    vec![
        mcp_json_patch(repo.join(".mcp.json"), bin),
        claude_shaped_patch(
            repo.join(".claude/settings.json"),
            bin,
            "Edit|Write|MultiEdit|NotebookEdit",
        ),
    ]
}

fn claude_user(home: &Path, bin: &str) -> Vec<Patch> {
    vec![
        mcp_json_patch(home.join(".claude.json"), bin),
        claude_shaped_patch(
            home.join(".claude/settings.json"),
            bin,
            "Edit|Write|MultiEdit|NotebookEdit",
        ),
    ]
}

fn cursor_project(repo: &Path, bin: &str) -> Vec<Patch> {
    vec![
        cursor_mcp_patch(repo.join(".cursor/mcp.json"), bin),
        cursor_hooks_patch(repo.join(".cursor/hooks.json"), bin),
    ]
}

fn cursor_user(home: &Path, bin: &str) -> Vec<Patch> {
    vec![
        cursor_mcp_patch(home.join(".cursor/mcp.json"), bin),
        cursor_hooks_patch(home.join(".cursor/hooks.json"), bin),
    ]
}

fn codex_hooks_patch(path: PathBuf, bin: &str) -> Patch {
    Patch::Json(
        path,
        json!({"hooks": hooks_codex(bin, "Edit|Write|NotebookEdit|apply_patch|edit|write")}),
    )
}

fn codex_project(repo: &Path, bin: &str) -> Vec<Patch> {
    vec![
        codex_mcp_patch(repo.join(".codex/config.toml"), bin),
        codex_hooks_patch(repo.join(".codex/hooks.json"), bin),
    ]
}

fn codex_user(home: &Path, bin: &str) -> Vec<Patch> {
    vec![
        codex_mcp_patch(home.join(".codex/config.toml"), bin),
        codex_hooks_patch(home.join(".codex/hooks.json"), bin),
    ]
}

fn gemini_project(repo: &Path, bin: &str) -> Vec<Patch> {
    vec![gemini_settings_patch(
        repo.join(".gemini/settings.json"),
        bin,
    )]
}

fn gemini_user(home: &Path, bin: &str) -> Vec<Patch> {
    vec![gemini_settings_patch(
        home.join(".gemini/settings.json"),
        bin,
    )]
}

fn copilot_project(repo: &Path, bin: &str) -> Vec<Patch> {
    vec![
        copilot_mcp_patch(repo.join(".mcp.json"), bin),
        copilot_hooks_patch(repo.join(".github/hooks/cogz.json"), bin),
    ]
}

fn copilot_user(home: &Path, bin: &str) -> Vec<Patch> {
    vec![
        copilot_mcp_patch(home.join(".copilot/mcp-config.json"), bin),
        copilot_hooks_patch(home.join(".copilot/hooks/cogz.json"), bin),
    ]
}

fn devin_project(repo: &Path, bin: &str) -> Vec<Patch> {
    vec![
        mcp_json_patch(repo.join(".devin/mcp_config.json"), bin),
        // .devin/hooks.v1.json IS the hooks object — no wrapper key.
        Patch::Json(repo.join(".devin/hooks.v1.json"), hooks_devin(bin)),
    ]
}

fn devin_user(home: &Path, bin: &str) -> Vec<Patch> {
    vec![
        mcp_json_patch(home.join(".config/devin/mcp_config.json"), bin),
        // config.json nests the same map under the "hooks" key.
        Patch::Json(
            home.join(".config/devin/config.json"),
            json!({"hooks": hooks_devin(bin)}),
        ),
    ]
}

pub(super) const HARNESSES: &[Harness] = &[
    Harness {
        id: "claude-code",
        repo_markers: &[".claude", ".mcp.json"],
        home_markers: &[".claude", ".claude.json"],
        project_patches: claude_project,
        user_patches: claude_user,
    },
    Harness {
        id: "cursor",
        repo_markers: &[".cursor"],
        home_markers: &[".cursor"],
        project_patches: cursor_project,
        user_patches: cursor_user,
    },
    Harness {
        id: "codex",
        repo_markers: &[".codex"],
        home_markers: &[".codex"],
        project_patches: codex_project,
        user_patches: codex_user,
    },
    Harness {
        id: "gemini",
        repo_markers: &[".gemini"],
        home_markers: &[".gemini"],
        project_patches: gemini_project,
        user_patches: gemini_user,
    },
    Harness {
        id: "copilot",
        repo_markers: &[".github/hooks"],
        home_markers: &[".copilot"],
        project_patches: copilot_project,
        user_patches: copilot_user,
    },
    Harness {
        id: "devin",
        repo_markers: &[".devin"],
        home_markers: &[".config/devin"],
        project_patches: devin_project,
        user_patches: devin_user,
    },
];

/// All valid harness ids, for errors and help text.
pub fn harness_ids() -> Vec<&'static str> {
    HARNESSES.iter().map(|h| h.id).collect()
}

/// Detect installed harnesses from marker files/dirs in the repo or
/// the user's home. Returns harness ids, in table order.
pub fn detect(repo: &Path, home: &Path) -> Vec<&'static str> {
    HARNESSES
        .iter()
        .filter(|h| {
            h.repo_markers.iter().any(|m| repo.join(m).exists())
                || h.home_markers.iter().any(|m| home.join(m).exists())
        })
        .map(|h| h.id)
        .collect()
}
