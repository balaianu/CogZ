//! Claude-shaped hook payload builders shared by the harnesses that
//! speak matcher-group hook config (Claude Code, Codex, Devin).

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::harnesses::Patch;
use super::shell_argv0;

fn hook_cmd(bin: &str, args: &str) -> String {
    format!("{} capture-event {args}", shell_argv0(bin))
}

/// Claude-shaped matcher-group entry. `timeout` is in seconds for
/// Claude Code, Codex, and Devin.
fn hook_entry(command: &str, timeout: u64, matcher: &str) -> Value {
    let mut group = Map::new();
    if !matcher.is_empty() {
        group.insert("matcher".into(), Value::String(matcher.to_string()));
    }
    group.insert(
        "hooks".into(),
        json!([{"type": "command", "command": command, "timeout": timeout}]),
    );
    Value::Object(group)
}

fn hook_entries(bin: &str, args: &str, timeout: u64, matcher: &str) -> Value {
    json!([hook_entry(
        &hook_cmd(bin, &format!("{args} --hook-json")),
        timeout,
        matcher
    )])
}

/// Canonical PascalCase event map (Claude Code shape — the de facto
/// standard Codex and Devin also speak). `edit_matcher` filters
/// PostToolUse down to file-writing tools for the file_save event.
/// PostCompact runs record-only: neither Claude's nor Codex's
/// PostCompact output schema can inject context, so a populated
/// emission would fail validation — SessionStart refires on
/// compaction (source=compact) and carries re-injection instead.
fn hooks_canonical(bin: &str, edit_matcher: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(
        "SessionStart".into(),
        hook_entries(bin, "session_start", 15, ""),
    );
    map.insert(
        "UserPromptSubmit".into(),
        hook_entries(bin, "prompt_submit", 15, ""),
    );
    map.insert(
        "PostToolUse".into(),
        json!([
            hook_entry(
                &hook_cmd(bin, "post_tool_use --hook-json --fts-only"),
                10,
                ""
            ),
            hook_entry(
                &hook_cmd(bin, "file_save --hook-json --fts-only"),
                20,
                edit_matcher
            ),
        ]),
    );
    map.insert(
        "SessionEnd".into(),
        hook_entries(bin, "session_end --fts-only", 30, ""),
    );
    map.insert("Stop".into(), hook_entries(bin, "stop --fts-only", 5, ""));
    map.insert(
        "PostCompact".into(),
        hook_entries(bin, "session_start --fts-only --record-only", 15, ""),
    );
    map
}

/// Devin speaks the canonical map but calls the compaction event
/// PostCompaction and uses lowercase tool names in matchers. Unlike
/// the canonical compact entry, PostCompaction keeps its emission —
/// Devin documents it for post-compaction re-injection, and the
/// stdin `hook_event_name` echo tags the output with the event the
/// consumer actually fired.
pub(super) fn hooks_devin(bin: &str) -> Value {
    let mut hooks = hooks_canonical(bin, "edit|write|notebook_edit");
    hooks.remove("PostCompact");
    hooks.insert(
        "PostCompaction".into(),
        hook_entries(bin, "session_start --fts-only", 15, ""),
    );
    Value::Object(hooks)
}

/// Codex speaks the canonical map but clamps SessionEnd hooks to 3
/// seconds max (1s default) — consolidation runs best-effort inside
/// the cap.
pub(super) fn hooks_codex(bin: &str, edit_matcher: &str) -> Value {
    let mut hooks = hooks_canonical(bin, edit_matcher);
    hooks.insert(
        "SessionEnd".into(),
        hook_entries(bin, "session_end --fts-only", 3, ""),
    );
    Value::Object(hooks)
}

pub(super) fn claude_shaped_patch(path: PathBuf, bin: &str, edit_matcher: &str) -> Patch {
    Patch::Json(path, json!({"hooks": hooks_canonical(bin, edit_matcher)}))
}
