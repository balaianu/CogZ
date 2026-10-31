//! Tests for `cogz configure` — file plans, harness payload shapes,
//! detection, and argv[0] repair. Merge/backup semantics live in
//! `tests_merge`.

use super::*;
use serde_json::json;
use std::fs;

fn dirs() -> (tempfile::TempDir, tempfile::TempDir) {
    (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn each_harness_writes_mcp_entry() {
    let (repo, home) = dirs();
    for id in harness_ids() {
        let out = run(repo.path(), id, Scope::Project, home.path(), "cogz").unwrap();
        assert!(out.contains("wrote"), "{id} should write files:\n{out}");
        let expected = match id {
            "claude-code" | "copilot" => repo.path().join(".mcp.json"),
            "cursor" => repo.path().join(".cursor/mcp.json"),
            "codex" => repo.path().join(".codex/config.toml"),
            "gemini" => repo.path().join(".gemini/settings.json"),
            "devin" => repo.path().join(".devin/mcp_config.json"),
            other => panic!("unmapped harness {other}"),
        };
        assert!(expected.exists(), "{id} MCP file missing: {expected:?}");
    }
}

#[test]
fn gemini_writes_mcp_and_hooks_to_one_file() {
    let (repo, home) = dirs();
    run(repo.path(), "gemini", Scope::Project, home.path(), "cogz").unwrap();
    let settings = read_json(&repo.path().join(".gemini/settings.json"));
    assert_eq!(settings["mcpServers"]["cogz"]["command"], "cogz");
    assert!(settings["hooks"]["SessionStart"].is_array());
    assert!(settings["hooks"]["AfterTool"].is_array());
    // Gemini-specific names — UserPromptSubmit/Stop do not exist there.
    assert!(settings["hooks"]["BeforeAgent"].is_array());
    assert!(settings["hooks"]["AfterAgent"].is_array());
    // Gemini timeouts are milliseconds, not seconds.
    let entry = &settings["hooks"]["SessionStart"][0]["hooks"][0];
    assert_eq!(entry["timeout"], json!(15000));
}

#[test]
fn cursor_writes_flat_versioned_hooks() {
    let (repo, home) = dirs();
    run(repo.path(), "cursor", Scope::Project, home.path(), "cogz").unwrap();
    let hooks = read_json(&repo.path().join(".cursor/hooks.json"));
    assert_eq!(hooks["version"], json!(1));
    // Flat {command, timeout} entries, camelCase event names.
    let entry = &hooks["hooks"]["beforeSubmitPrompt"][0];
    assert!(entry["command"].as_str().unwrap().contains("prompt_submit"));
    // Cursor reads additional_context, not hookSpecificOutput.
    assert!(
        entry["command"]
            .as_str()
            .unwrap()
            .contains("--hook-format cursor")
    );
    assert!(entry.get("hooks").is_none(), "entries must be flat");
    // Native file-edit event; no postToolUse matcher needed.
    assert!(hooks["hooks"]["afterFileEdit"].is_array());
    assert!(hooks["hooks"]["stop"].is_array());
}

#[test]
fn copilot_writes_versioned_hookset_file() {
    let (repo, home) = dirs();
    run(repo.path(), "copilot", Scope::Project, home.path(), "cogz").unwrap();
    let hooks = read_json(&repo.path().join(".github/hooks/cogz.json"));
    assert_eq!(hooks["version"], json!(1));
    // PascalCase names → Copilot sends snake_case stdin payloads.
    assert!(hooks["hooks"]["SessionStart"].is_array());
    assert!(hooks["hooks"]["UserPromptSubmit"].is_array());
    assert!(hooks["hooks"]["Stop"].is_array());
    // Flat entries use `command` (cross-platform) + `timeoutSec`.
    let entry = &hooks["hooks"]["SessionStart"][0];
    assert_eq!(entry["type"], json!("command"));
    assert_eq!(entry["timeoutSec"], json!(15));
    assert!(entry["command"].as_str().unwrap().contains("session_start"));
    // Copilot reads top-level additionalContext, not hookSpecificOutput.
    assert!(
        entry["command"]
            .as_str()
            .unwrap()
            .contains("--hook-format copilot")
    );
}

/// Cursor requires `"type": "stdio"` on stdio MCP servers — without it
/// the server can fail to register silently.
#[test]
fn cursor_mcp_entry_declares_stdio_type() {
    let (repo, home) = dirs();
    run(repo.path(), "cursor", Scope::Project, home.path(), "cogz").unwrap();
    let mcp = read_json(&repo.path().join(".cursor/mcp.json"));
    let entry = &mcp["mcpServers"]["cogz"];
    assert_eq!(entry["type"], "stdio");
    assert_eq!(entry["command"], "cogz");
    assert_eq!(entry["args"], json!(["mcp-stdio"]));
}

/// Copilot's MCP shape is `type: "local"` + `tools` allowlist — without
/// `tools` the server exposes nothing.
#[test]
fn copilot_mcp_entry_declares_local_type_and_tools() {
    let (repo, home) = dirs();
    run(repo.path(), "copilot", Scope::Project, home.path(), "cogz").unwrap();
    let mcp = read_json(&repo.path().join(".mcp.json"));
    let entry = &mcp["mcpServers"]["cogz"];
    assert_eq!(entry["type"], "local");
    assert_eq!(entry["command"], "cogz");
    assert_eq!(entry["tools"], json!(["*"]));
}

/// Codex clamps SessionEnd (and Interrupt) hooks to 3 seconds max —
/// the canonical 30s would be rejected or truncate consolidation.
#[test]
fn codex_session_end_respects_3s_cap() {
    let (repo, home) = dirs();
    run(repo.path(), "codex", Scope::Project, home.path(), "cogz").unwrap();
    let hooks = read_json(&repo.path().join(".codex/hooks.json"));
    let end = hooks["hooks"]["SessionEnd"].as_array().unwrap();
    assert_eq!(end[0]["hooks"][0]["timeout"], json!(3));
}

#[test]
fn devin_includes_postcompaction() {
    let (repo, home) = dirs();
    run(repo.path(), "devin", Scope::Project, home.path(), "cogz").unwrap();
    let hooks = read_json(&repo.path().join(".devin/hooks.v1.json"));
    // hooks.v1.json is the hooks object itself — event names sit at
    // the top level, not under a "hooks" key.
    assert!(
        hooks.get("hooks").is_none(),
        "hooks.v1.json must be the bare event map"
    );
    let post = hooks["PostCompaction"].as_array().unwrap();
    assert!(
        post[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("session_start")
    );
    // Devin's compact event injects — no record-only suppression.
    assert!(
        !post[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--record-only")
    );
}

#[test]
fn user_scope_writes_under_home() {
    let (repo, home) = dirs();
    run(repo.path(), "devin", Scope::User, home.path(), "cogz").unwrap();
    assert!(home.path().join(".config/devin/mcp_config.json").exists());
    let cfg = read_json(&home.path().join(".config/devin/config.json"));
    assert!(cfg["hooks"]["SessionStart"].is_array());
    assert!(!repo.path().join(".devin").exists());
}

/// Hook runtimes and MCP spawners often run without `~/.local/bin` on
/// PATH — a bare `cogz` command fails with exit 127. `bin` flows
/// verbatim into generated hook and MCP commands, so passing an
/// absolute path wires a runnable invocation.
#[test]
fn absolute_bin_propagates_to_generated_commands() {
    let (repo, home) = dirs();
    let bin = "/home/u/.local/bin/cogz";
    run(repo.path(), "devin", Scope::Project, home.path(), bin).unwrap();

    let hooks = read_json(&repo.path().join(".devin/hooks.v1.json"));
    let cmd = hooks["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(cmd.starts_with(bin), "hook must invoke resolved bin: {cmd}");
    assert!(cmd.contains("capture-event session_start"));

    let mcp = read_json(&repo.path().join(".devin/mcp_config.json"));
    assert_eq!(mcp["mcpServers"]["cogz"]["command"], bin);
}

#[test]
fn detect_finds_home_and_repo_markers() {
    let (repo, home) = dirs();
    assert!(detect(repo.path(), home.path()).is_empty());

    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::create_dir_all(repo.path().join(".devin")).unwrap();
    let found = detect(repo.path(), home.path());
    assert_eq!(found, vec!["claude-code", "devin"]);
}

#[test]
fn auto_resolves_detected_harnesses() {
    let (repo, home) = dirs();
    fs::create_dir_all(home.path().join(".cursor")).unwrap();
    let out = run(repo.path(), "auto", Scope::Project, home.path(), "cogz").unwrap();
    assert!(out.starts_with("cursor:"));
    assert!(repo.path().join(".cursor/mcp.json").exists());
}

#[test]
fn unknown_harness_and_empty_auto_error() {
    let (repo, home) = dirs();
    assert!(run(repo.path(), "nvim", Scope::Project, home.path(), "cogz").is_err());
    assert!(run(repo.path(), "auto", Scope::Project, home.path(), "cogz").is_err());
    assert!(
        run(
            repo.path(),
            "auto,devin",
            Scope::Project,
            home.path(),
            "cogz"
        )
        .is_err()
    );
}

#[test]
fn corrupt_existing_file_errors_without_touching_it() {
    let (repo, home) = dirs();
    let mcp = repo.path().join(".mcp.json");
    fs::write(&mcp, "{not json").unwrap();
    let result = run(
        repo.path(),
        "claude-code",
        Scope::Project,
        home.path(),
        "cogz",
    );
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&mcp).unwrap(), "{not json");
}

/// Configs generated (or hand-wired) before `bin` was resolved carry
/// bare `cogz` commands that die under PATH-less hook runtimes. Dedup
/// counts them as wired, so reconfigure must repair argv[0] in place —
/// flags preserved, foreign commands untouched.
#[test]
fn rerun_repairs_bare_cogz_hook_commands() {
    let (repo, home) = dirs();
    let cfg_dir = home.path().join(".config/devin");
    fs::create_dir_all(&cfg_dir).unwrap();
    let cfg = cfg_dir.join("config.json");
    fs::write(
        &cfg,
        r#"{"hooks":{"SessionStart":[{"matcher":"","hooks":[
            {"type":"command","command":"cogz capture-event session_start --hook-json","timeout":5},
            {"type":"command","command":"bash ~/hooks/other.sh","timeout":10}
        ]}]}}"#,
    )
    .unwrap();

    let bin = "/home/u/.local/bin/cogz";
    run(repo.path(), "devin", Scope::User, home.path(), bin).unwrap();

    let merged = read_json(&cfg);
    let entries = merged["hooks"]["SessionStart"][0]["hooks"]
        .as_array()
        .unwrap();
    let cmds: Vec<&str> = entries
        .iter()
        .map(|h| h["command"].as_str().unwrap())
        .collect();
    let want = format!("{bin} capture-event session_start --hook-json");
    assert!(cmds.contains(&want.as_str()), "repaired: {cmds:?}");
    assert_eq!(
        cmds.iter().filter(|c| c.contains("capture-event")).count(),
        1,
        "repair must not duplicate"
    );
    assert!(cmds.contains(&"bash ~/hooks/other.sh"));
}

#[test]
fn bare_cogz_mcp_command_is_upgraded() {
    let (repo, home) = dirs();
    let mcp = repo.path().join(".mcp.json");
    fs::write(
        &mcp,
        r#"{"mcpServers":{"cogz":{"command":"cogz","args":["mcp-stdio"]}}}"#,
    )
    .unwrap();

    run(
        repo.path(),
        "claude-code",
        Scope::Project,
        home.path(),
        "/opt/bin/cogz",
    )
    .unwrap();

    let merged = read_json(&mcp);
    assert_eq!(merged["mcpServers"]["cogz"]["command"], "/opt/bin/cogz");
    assert_eq!(merged["mcpServers"]["cogz"]["args"], json!(["mcp-stdio"]));
}

#[test]
fn bare_cogz_toml_mcp_command_is_upgraded() {
    let (repo, home) = dirs();
    let codex = repo.path().join(".codex");
    fs::create_dir_all(&codex).unwrap();
    let cfg = codex.join("config.toml");
    fs::write(
        &cfg,
        "[mcp_servers.cogz]\ncommand = \"cogz\"\nargs = [\"mcp-stdio\"]\n",
    )
    .unwrap();

    run(
        repo.path(),
        "codex",
        Scope::Project,
        home.path(),
        "/opt/bin/cogz",
    )
    .unwrap();

    let merged: toml::Value = toml::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
    let entry = &merged["mcp_servers"]["cogz"];
    assert_eq!(entry["command"].as_str().unwrap(), "/opt/bin/cogz");
    assert_eq!(entry["args"][0].as_str().unwrap(), "mcp-stdio");
}

/// A `bin` containing whitespace (`C:\Users\<name>\…`) must be
/// double-quoted in hook command strings so shell-splitting keeps it
/// one token; MCP `command` fields take the raw path (direct argv[0]).
#[test]
fn bin_with_spaces_is_quoted_in_hooks_raw_in_mcp() {
    let (repo, home) = dirs();
    let bin = "C:\\Users\\Jane Doe\\tools\\cogz.exe";
    run(repo.path(), "devin", Scope::Project, home.path(), bin).unwrap();

    let hooks = read_json(&repo.path().join(".devin/hooks.v1.json"));
    let cmd = hooks["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let quoted = format!("\"{bin}\"");
    assert!(cmd.starts_with(&quoted), "spaced bin unquoted: {cmd}");

    let mcp = read_json(&repo.path().join(".devin/mcp_config.json"));
    assert_eq!(mcp["mcpServers"]["cogz"]["command"], bin);

    // Repair path: a pre-existing bare-`cogz` entry upgrades to the
    // same quoted command rather than duplicating alongside it.
    let cfg_dir = home.path().join(".config/devin");
    fs::create_dir_all(&cfg_dir).unwrap();
    let cfg = cfg_dir.join("config.json");
    fs::write(
        &cfg,
        r#"{"hooks":{"SessionStart":[{"matcher":"","hooks":[
            {"type":"command","command":"cogz capture-event session_start --hook-json","timeout":5}
        ]}]}}"#,
    )
    .unwrap();
    run(repo.path(), "devin", Scope::User, home.path(), bin).unwrap();
    let merged = read_json(&cfg);
    let entries = merged["hooks"]["SessionStart"][0]["hooks"]
        .as_array()
        .unwrap();
    let cmds: Vec<&str> = entries
        .iter()
        .filter_map(|h| h["command"].as_str())
        .filter(|c| c.contains("capture-event"))
        .collect();
    assert_eq!(cmds.len(), 1, "upgrade must not duplicate: {cmds:?}");
    assert_eq!(
        cmds[0],
        format!("\"{bin}\" capture-event session_start --hook-json")
    );
}
