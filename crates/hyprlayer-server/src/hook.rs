//! `hyprlayer-server hook <engine> [Event]`: a claude or codex hook that reports one signal and gets out of the way.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_models::{AgentSignal, ClaudeWorktreeAnswer, ClaudeWorktreeEvent};
use serde_json::Value;

use crate::client::RpcClient;

/// A slow or hung server must never stall the agent's turn.
const REPORT_BUDGET: Duration = Duration::from_millis(500);
/// Leaves 30 s of claude's 120 s `WorktreeCreate` timeout for the local fallback.
const CREATE_BUDGET: Duration = Duration::from_secs(90);
const TOOL_TARGET_CHARS: usize = 80;
const LAST_REPLY_BYTES: usize = 4 * 1024;
const PROMPT_BYTES: usize = 2 * 1024;
/// claude's answer that skips its "Switch model?" dialog.
const ALLOW_SWITCH: &str =
    r#"{"hookSpecificOutput":{"hookEventName":"PreModelSwitch","permissionDecision":"allow"}}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEngine {
    Claude,
    Codex,
}

impl HookEngine {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(HookEngine::Claude),
            "codex" => Some(HookEngine::Codex),
            _ => None,
        }
    }
}

/// Never logs, and exits 0 unless `WorktreeCreate` can make no worktree at all: a failing hook
/// would surface in the agent's TUI.
pub async fn run_hook(engine: HookEngine, event: Option<String>) -> i32 {
    // codex parses stdout as JSON after exit 0; a closed stdout must not panic.
    let ack = || {
        if engine == HookEngine::Codex {
            let _ = writeln!(std::io::stdout(), "{{}}");
        }
    };
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    // claude takes the last stdout line as the worktree, so even a tokenless hook must answer.
    if engine == HookEngine::Claude && event.as_deref() == Some("WorktreeCreate") {
        return create_worktree(env("HYPRLAYER_SOCKET"), env("HYPRLAYER_AGENT_TOKEN")).await;
    }
    let (Some(token), Some(socket)) = (env("HYPRLAYER_AGENT_TOKEN"), env("HYPRLAYER_SOCKET"))
    else {
        ack();
        return 0;
    };
    if engine == HookEngine::Claude && event.as_deref() == Some("PreModelSwitch") {
        let _ = read_stdin();
        return allow_driven_switch(PathBuf::from(socket), token).await;
    }
    let Some(signal) = read_stdin().and_then(|raw| trim(&raw, engine, event)) else {
        ack();
        return 0;
    };
    let _ = tokio::time::timeout(REPORT_BUDGET, report(PathBuf::from(socket), token, signal)).await;
    ack();
    0
}

async fn report(socket: PathBuf, token: String, signal: AgentSignal) {
    let Ok(client) = RpcClient::connect_socket(&socket).await else {
        return;
    };
    drop(client.take_events());
    let _ = client.report_agent_signal(token, signal).await;
}

/// Allows only a switch the status bar is driving; silence leaves the user's own switches to claude.
async fn allow_driven_switch(socket: PathBuf, token: String) -> i32 {
    let ask = async {
        let client = RpcClient::connect_socket(&socket).await.ok()?;
        drop(client.take_events());
        client.model_switch_pending(token).await.ok()
    };
    if let Ok(Some(true)) = tokio::time::timeout(REPORT_BUDGET, ask).await {
        let _ = writeln!(std::io::stdout(), "{ALLOW_SWITCH}");
    }
    0
}

async fn create_worktree(socket: Option<String>, token: Option<String>) -> i32 {
    let payload: Option<Value> = read_stdin().and_then(|raw| serde_json::from_slice(&raw).ok());
    let text = |key: &str| payload.as_ref()?.get(key)?.as_str().map(str::to_string);
    let (Some(name), Some(cwd)) = (text("name"), text("cwd")) else {
        eprintln!("hyprlayer: the WorktreeCreate payload has no name or cwd");
        return 1;
    };
    let event = ClaudeWorktreeEvent::Create {
        name: name.clone(),
        cwd: cwd.clone(),
        session_id: text("session_id"),
    };
    let ask = async {
        let client = RpcClient::connect_socket(Path::new(&socket?)).await.ok()?;
        drop(client.take_events());
        client.claude_worktree(token?, event).await.ok()
    };
    let path = match tokio::time::timeout(CREATE_BUDGET, ask).await {
        Ok(Some(ClaudeWorktreeAnswer::Path(path))) => path,
        _ => match claude_layout_worktree(Path::new(&cwd), &name) {
            Ok(path) => path,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
    };
    let _ = writeln!(std::io::stdout(), "{path}");
    0
}

/// The worktree claude makes with no hook installed: `worktree-<name>` off the origin default.
fn claude_layout_worktree(cwd: &Path, name: &str) -> Result<String, String> {
    let git = |dir: &Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map_err(|e| format!("git: {e}"))?;
        match out.status.success() {
            true => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
            false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        }
    };
    let top = PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"])?);
    let path = top.join(".claude").join("worktrees").join(name);
    let path_str = path.to_string_lossy().into_owned();
    if path.is_dir() {
        return Ok(path_str);
    }
    let branch = format!("worktree-{name}");
    let base =
        git(&top, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).unwrap_or_else(|_| "HEAD".into());
    let local = format!("refs/heads/{branch}");
    let add: Vec<&str> = match git(&top, &["rev-parse", "--verify", "--quiet", &local]) {
        Ok(_) => vec!["worktree", "add", &path_str, &branch],
        Err(_) => vec!["worktree", "add", "-b", &branch, &path_str, &base],
    };
    git(&top, &add)?;
    Ok(path_str)
}

pub(crate) fn read_stdin() -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    std::io::stdin().read_to_end(&mut raw).ok()?;
    Some(raw)
}

/// The fields the fold reads, cut to size; `None` for a payload that isn't a JSON object.
fn trim(raw: &[u8], engine: HookEngine, event: Option<String>) -> Option<AgentSignal> {
    let payload: Value = serde_json::from_slice(raw).ok()?;
    let payload = payload.as_object()?;
    let text = |key: &str| payload.get(key)?.as_str().map(str::to_string);
    let event = event.or_else(|| text("hook_event_name"))?;
    // `StopFailure`'s kind (`authentication_failed`), else the first line of its details.
    let failure = || {
        text("error")
            .or_else(|| text("error_details"))
            .and_then(|e| {
                e.lines()
                    .next()
                    .map(|l| l.trim().chars().take(TOOL_TARGET_CHARS).collect())
            })
    };
    let tool_target = payload.get("tool_input").and_then(|input| {
        ["command", "file_path", "pattern", "url"]
            .iter()
            .find_map(|key| input.get(key)?.as_str())
            .map(|target| target.chars().take(TOOL_TARGET_CHARS).collect())
    });
    Some(AgentSignal {
        event,
        session_id: text("session_id"),
        source: text("source"),
        reason: text("reason").or_else(failure),
        tool_name: text("tool_name"),
        tool_target,
        notification_type: text("notification_type"),
        last_assistant_message: text("last_assistant_message")
            .map(|m| truncate(m, LAST_REPLY_BYTES)),
        turn_id: text("turn_id"),
        prompt: text("prompt").map(|p| truncate(p, PROMPT_BYTES)),
        permission_mode: text("permission_mode"),
        effort: payload
            .get("effort")
            .and_then(|e| e.get("level")?.as_str())
            .map(str::to_string),
        model: text("model"),
        to_model: text("to_model"),
        // Only codex's rollout carries what its hooks lack.
        transcript_path: text("transcript_path").filter(|_| engine == HookEngine::Codex),
        written_paths: written_paths(payload),
    })
}

fn written_paths(payload: &serde_json::Map<String, Value>) -> Vec<String> {
    let tool = payload.get("tool_name").and_then(Value::as_str);
    let (Some("Write" | "Edit" | "MultiEdit"), Some(input)) = (tool, payload.get("tool_input"))
    else {
        return Vec::new();
    };
    let cwd = payload.get("cwd").and_then(Value::as_str).map(Path::new);
    let edits = input.get("edits").and_then(Value::as_array);
    let mut paths: Vec<String> = std::iter::once(input.get("file_path"))
        .chain(edits.into_iter().flatten().map(|e| e.get("file_path")))
        .filter_map(|p| p?.as_str())
        // Without its `cwd` a relative path would resolve against the server's.
        .filter_map(|p| match cwd {
            Some(cwd) => Some(cwd.join(p).to_string_lossy().into_owned()),
            None => Path::new(p).is_absolute().then(|| p.to_string()),
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn truncate(mut s: String, max_bytes: usize) -> String {
    if s.len() > max_bytes {
        let cut = (0..=max_bytes)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        s.truncate(cut);
    }
    s
}
