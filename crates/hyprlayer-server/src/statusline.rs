//! `hyprlayer-server statusline claude`: a hooked claude's statusLine, which reports model, effort
//! and context, then runs the user's own statusLine with the same input and prints what it prints.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use hyprlayer_models::{AgentStatusReport, PtyAgentContext};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::client::RpcClient;

/// claude aborts a statusLine run when the next one starts; the report must not hold up the user's.
const REPORT_BUDGET: Duration = Duration::from_millis(500);

/// Always exits 0: a failing statusLine would blank the user's own.
pub async fn run_statusline() -> i32 {
    let Some(raw) = crate::hook::read_stdin() else {
        return 0;
    };
    let payload: Option<Value> = serde_json::from_slice(&raw).ok();
    let report = async {
        if let Some(report) = payload.as_ref().and_then(report_of) {
            let _ = tokio::time::timeout(REPORT_BUDGET, send(report)).await;
        }
    };
    let (_, printed) = tokio::join!(report, user_status_line(payload.as_ref(), &raw));
    if let Some(printed) = printed {
        let _ = std::io::stdout().write_all(&printed);
    }
    0
}

async fn send(report: AgentStatusReport) {
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let (Some(token), Some(socket)) = (env("HYPRLAYER_AGENT_TOKEN"), env("HYPRLAYER_SOCKET"))
    else {
        return;
    };
    let Ok(client) = RpcClient::connect_socket(&PathBuf::from(socket)).await else {
        return;
    };
    drop(client.take_events());
    let _ = client.report_agent_status(token, report).await;
}

/// Exactly the fields claude filled; a null `used_percentage` (no usage yet, or after `/clear`) is no context.
fn report_of(payload: &Value) -> Option<AgentStatusReport> {
    payload.as_object()?;
    let text = |pointer: &str| payload.pointer(pointer)?.as_str().map(str::to_string);
    let context = payload.get("context_window").and_then(|w| {
        Some(PtyAgentContext {
            tokens: u32::try_from(w.get("total_input_tokens")?.as_u64()?).ok()?,
            window: u32::try_from(w.get("context_window_size")?.as_u64()?).ok()?,
            percent: Some(w.get("used_percentage")?.as_f64()?.round() as u32),
        })
    });
    Some(AgentStatusReport {
        model: text("/model/id"),
        effort: text("/effort/level"),
        context,
    })
}

/// The user's statusLine output, or `None` when they have none or it can't run.
async fn user_status_line(payload: Option<&Value>, stdin: &[u8]) -> Option<Vec<u8>> {
    let command = user_command(payload)?;
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&command);
        cmd
    };
    // claude's own convention there; raw so cmd sees the line as the user wrote it.
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").raw_arg(&command);
        cmd
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin).await;
    }
    Some(child.wait_with_output().await.ok()?.stdout)
}

/// claude's precedence: the project's local then shared settings, then the user's; the first `statusLine` wins.
fn user_command(payload: Option<&Value>) -> Option<String> {
    let project = payload
        .and_then(|p| {
            p.pointer("/workspace/project_dir")
                .or_else(|| p.pointer("/workspace/current_dir"))
                .or_else(|| p.get("cwd"))
        })
        .and_then(Value::as_str)
        .map(|dir| PathBuf::from(dir).join(".claude"));
    let user = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")));
    let files = project
        .iter()
        .flat_map(|dir| [dir.join("settings.local.json"), dir.join("settings.json")])
        .chain(user.map(|dir| dir.join("settings.json")));
    for file in files {
        let Some(settings) = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            continue;
        };
        let Some(line) = settings.get("statusLine") else {
            continue;
        };
        if line.get("type")?.as_str()? != "command" {
            return None;
        }
        return line.get("command")?.as_str().map(str::to_string);
    }
    None
}
