//! What a launch injects so an engine reports back and reaches the `hyprlayer` MCP server.

use std::path::Path;

pub const CLAUDE_HOOK_EVENTS: [&str; 14] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    // Fires instead of `Stop` when an API or auth error ends the turn.
    "StopFailure",
    "SessionEnd",
    "PostModelSwitch",
    // Asks the server rather than reports: it answers claude's "Switch model?" dialog for the bar.
    "PreModelSwitch",
    // Answers with a path: the server's workstream, else claude's own layout, made by the face.
    "WorktreeCreate",
    // Archive the workstream claude is done with: on a remove, or a subagent that changed nothing.
    "WorktreeRemove",
    "SubagentStop",
];

pub const CODEX_HOOK_EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
    "Interrupt",
    "SessionEnd",
];

/// The oldest codex every injected flag was probed on.
pub const CODEX_HOOKS_MIN: [u64; 3] = [0, 164, 0];

const HOOK_TIMEOUT_SECS: u32 = 5;
/// Above the face's 90 s budget for the server's create plus its local fallback.
const WORKTREE_CREATE_TIMEOUT_SECS: u32 = 120;

const MCP_SERVER: &str = "hyprlayer";
/// Above the 600 s cap on a wait, so codex never cuts one short.
const MCP_TOOL_TIMEOUT_SECS: u32 = 610;

/// Tools that only read, or only reach the user; safe to allow without a prompt.
pub const READ_TOOLS: [&str; 13] = [
    "whoami",
    "list_workstreams",
    "list_agents",
    "read_agent",
    "list_docs",
    "read_doc",
    "pr_status",
    "git_status",
    "script_output",
    "notify_user",
    "request_attention",
    "read_terminal",
    "wake_when",
];

/// claude permission rules naming every read tool of the `hyprlayer` server.
pub fn mcp_allow_rules() -> Vec<String> {
    READ_TOOLS
        .iter()
        .map(|tool| format!("mcp__{MCP_SERVER}__{tool}"))
        .collect()
}

/// The identity env the MCP process needs; the ctl path is its command, not its env.
fn mcp_env(identity_env: &[(String, String)]) -> impl Iterator<Item = &(String, String)> {
    identity_env
        .iter()
        .filter(|(k, _)| k != super::AGENT_ENV[0])
}

/// claude's `--mcp-config` JSON for the `hyprlayer` server.
pub fn claude_mcp_config(ctl: &Path, identity_env: &[(String, String)]) -> serde_json::Value {
    let env: serde_json::Map<String, serde_json::Value> = mcp_env(identity_env)
        .map(|(k, v)| (k.clone(), v.clone().into()))
        .collect();
    serde_json::json!({ "mcpServers": { MCP_SERVER: {
        "type": "stdio",
        "command": ctl.to_string_lossy(),
        "args": ["mcp"],
        "env": env,
    } } })
}

/// codex's `-c mcp_servers.hyprlayer.*` pairs; `None` when a value can't be a TOML literal.
pub fn codex_mcp_args(
    ctl: &Path,
    identity_env: &[(String, String)],
    read_only: bool,
) -> Option<Vec<String>> {
    let args = if read_only {
        "['mcp','--read-only']"
    } else {
        "['mcp']"
    };
    let env = mcp_env(identity_env)
        .map(|(k, v)| Some(format!("{k}={}", toml_literal(v)?)))
        .collect::<Option<Vec<_>>>()?
        .join(",");
    let key = format!("mcp_servers.{MCP_SERVER}");
    let pairs = [
        format!("{key}.command={}", toml_literal(ctl.to_str()?)?),
        format!("{key}.args={args}"),
        format!("{key}.env={{{env}}}"),
        format!("{key}.default_tools_approval_mode='approve'"),
        format!("{key}.tool_timeout_sec={MCP_TOOL_TIMEOUT_SECS}"),
    ];
    Some(
        pairs
            .into_iter()
            .flat_map(|pair| ["-c".to_string(), pair])
            .collect(),
    )
}

/// claude's `--settings` JSON: `<ctl> hook claude <Event>` for every event, the statusLine face, and the read tools allowed.
pub fn claude_settings(ctl: &Path) -> serde_json::Value {
    let ctl = quoted_ctl(ctl);
    let hooks: serde_json::Map<String, serde_json::Value> = CLAUDE_HOOK_EVENTS
        .iter()
        .map(|event| {
            let hook = serde_json::json!([{ "hooks": [{
                "type": "command",
                "command": format!("{ctl} hook claude {event}"),
                "timeout": match *event {
                    "WorktreeCreate" => WORKTREE_CREATE_TIMEOUT_SECS,
                    _ => HOOK_TIMEOUT_SECS,
                },
            }] }]);
            (event.to_string(), hook)
        })
        .collect();
    // Replaces the user's own statusLine for this launch; the face runs theirs in turn.
    let status_line =
        serde_json::json!({ "type": "command", "command": format!("{ctl} statusline claude") });
    serde_json::json!({
        "hooks": hooks,
        "statusLine": status_line,
        "permissions": { "allow": mcp_allow_rules() },
    })
}

/// A Lead's agent `hooks`: `<ctl> lead-guard <checkout>` before every file edit and shell command.
pub fn lead_guard_hooks(ctl: &Path, checkout: &str) -> serde_json::Value {
    let command = format!(
        "{} lead-guard {}",
        quoted_ctl(ctl),
        quoted_ctl(Path::new(checkout))
    );
    serde_json::json!({ "PreToolUse": [{
        "matcher": "Bash|Edit|Write|NotebookEdit",
        "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }],
    }] })
}

#[cfg(not(windows))]
fn quoted_ctl(ctl: &Path) -> String {
    crate::runtime::terminals::sh_quote(&ctl.to_string_lossy())
}

#[cfg(windows)]
fn quoted_ctl(ctl: &Path) -> String {
    format!("\"{}\"", ctl.display())
}

/// Runs `-c` hooks without codex's trust review; the TUI's flag, wherever the hooks are set.
pub const CODEX_HOOK_TRUST: &str = "--dangerously-bypass-hook-trust";

/// hyprlayer's bar replaces codex's footer; only `N% context left` remains, which no key hides.
/// TUI-side only: on an embedded launch any `-c` adds a startup warning to the footer instead.
pub const CODEX_REMOTE_FOOTER: [&str; 4] = [
    "-c",
    "tui.status_line=[]",
    "-c",
    "tui.keymap.composer.toggle_shortcuts=[]",
];

/// codex's `-c hooks.<Event>=…` pairs; `None` when the ctl path can't ride bare in a hook command.
pub fn codex_hook_args(ctl: &Path) -> Option<Vec<String>> {
    let ctl = ctl.to_str()?;
    // codex runs the hook command through a shell, unquoted, so globs would expand too.
    if ctl
        .chars()
        .any(|c| c.is_whitespace() || "$`\";&|<>()*?[".contains(c))
    {
        return None;
    }
    let mut args = Vec::with_capacity(CODEX_HOOK_EVENTS.len() * 2);
    for event in CODEX_HOOK_EVENTS {
        let command = toml_literal(&format!("{ctl} hook codex {event}"))?;
        args.push("-c".to_string());
        args.push(format!(
            "hooks.{event}=[{{hooks=[{{type='command',command={command}}}]}}]"
        ));
    }
    Some(args)
}

/// A TOML literal string, which keeps `"` out of argv (Windows PowerShell 5.1 strips it).
pub fn toml_literal(value: &str) -> Option<String> {
    (!value.contains(['\'', '\n', '\r'])).then(|| format!("'{value}'"))
}
