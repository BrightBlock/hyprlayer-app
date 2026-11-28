//! A spawner's `stop_agent` cancels its PTY agent's question or permission prompt. claude fires no
//! hook for that, so only the screen shows the agent idle again, and it takes messages once more.
//! A prompt still on screen keeps refusing them.
//!
//! The caller is `goal_stub`'s claude stand-in driven through `hyprlayer-server mcp`; its PTY
//! child is `agent_stub`'s PTY claude.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::models::{AgentControlLevel, PermissionMode};
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::agent_stub::{
    agent_env, env_var, install, launch_log, lines, set_engine, set_level, tool_failed, tool_text,
    type_line, McpDriver, PTY_CLAUDE,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(30);
/// Past the settle's 2 s of quiet, so a settle that would idle the agent has.
const PAST_SETTLE: Duration = Duration::from_secs(4);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_ask_cancelled() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("pty_ask_cancelled", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Asks").await;
    let ws = text(&view, "/item/id");
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    let lead = start_lead(&mut j, &c, &sandbox, &ws).await;
    let lead_env = agent_env(&sandbox, &lead);
    j.alias(env_var(&lead_env, "HYPRLAYER_AGENT_TOKEN"), "$LEAD_TOKEN");
    let mut mcp = McpDriver::start(&lead_env, false);
    mcp.initialize("2025-06-18").await;

    j.step("the lead spawns a hooked PTY claude");
    let spawned = mcp
        .call("spawn_agent", json!({ "terminal_engine": "claude-code" }))
        .await;
    let term = text(&parsed(&spawned), "/terminalId");
    j.alias(&term, "$PTY");
    let log = launch_log(&sandbox, &term);
    let stdin = log.join("stdin.log");
    let idle = reaches(&c, &ws, &term, "idle").await;
    j.guard(
        "it starts idle",
        idle,
        json!({ "state": pty_state(&c, &ws, &term).await }),
    );

    for (line, asking) in [
        ("question", "awaiting_question"),
        ("ask", "awaiting_permission"),
    ] {
        j.step(&format!("{line}: cancelled by stop_agent"));
        type_line(&c, &term, line).await;
        let opened = reaches(&c, &ws, &term, asking).await;
        let before = lines(&stdin).len();
        let refused = refusal(
            &mcp.call(
                "message_agent",
                json!({ "terminal_id": term, "text": "E2E_WHILE_ASKING" }),
            )
            .await,
        );
        j.guard(
            &format!("while it reads {asking}, a message is refused and nothing reaches the TUI"),
            opened && refused.contains("asking its user something") && lines(&stdin).len() == before,
            json!({ "opened": opened, "refusal": refused, "typedSince": lines(&stdin).len() - before }),
        );

        let stopped = mcp.call("stop_agent", json!({ "terminal_id": term })).await;
        let idled = reaches(&c, &ws, &term, "idle").await;
        let hooks = lines(&log.join("hooks.log"));
        j.guard(
            "with no hook after the Esc, the agent's input box on screen settles it idle",
            !tool_failed(&stopped) && idled,
            json!({ "stopped": tool_text(&stopped), "state": pty_state(&c, &ws, &term).await, "lastHook": hooks.last() }),
        );

        let text = format!("E2E_AFTER_{}", line.to_uppercase());
        let sent = mcp
            .call(
                "message_agent",
                json!({ "terminal_id": term, "text": text }),
            )
            .await;
        let closing = format!("{text}\x1b[201~");
        let typed = eventually(IO, || lines(&stdin).last() == Some(&closing)).await;
        j.guard(
            "a message after the cancel is typed into the TUI",
            !tool_failed(&sent) && typed,
            json!({ "sent": tool_text(&sent), "typed": typed }),
        );
    }

    j.step("a question still on screen keeps refusing messages");
    type_line(&c, &term, "question").await;
    let opened = reaches(&c, &ws, &term, "awaiting_question").await;
    // A key that leaves the prompt up arms the settle, which must not idle it.
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("x"))
        .await
        .expect("terminal_write");
    tokio::time::sleep(PAST_SETTLE).await;
    let state = pty_state(&c, &ws, &term).await;
    let refused = refusal(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": term, "text": "E2E_STILL_ASKING" }),
        )
        .await,
    );
    j.guard(
        "past the settle it still reads awaiting_question, and a message is refused",
        opened
            && state.as_deref() == Some("awaiting_question")
            && refused.contains("asking its user something"),
        json!({ "state": state, "refusal": refused }),
    );

    j.finish();
}

async fn start_lead(j: &mut Journey, c: &RpcClient, sandbox: &Sandbox, ws: &str) -> String {
    let (session, _prompt, _stream_id, _events) = c
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_START".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode: Some(PermissionMode::Default),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$LEAD");
    j.call(
        c,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let done = || async {
        c.get_session_log(session.id.clone())
            .await
            .expect("get_session_log")
            .iter()
            .any(|e| matches!(e.event, SessionEvent::TurnDone { .. }))
    };
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline && !done().await {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eventually(IO, || {
        !env_var(&agent_env(sandbox, &session.id), "HYPRLAYER_AGENT_TOKEN").is_empty()
    })
    .await;
    session.id
}

fn parsed(result: &Value) -> Value {
    serde_json::from_str(&tool_text(result)).unwrap_or(Value::Null)
}

/// A refused tool's first line, `<code>: <message>`; `ok` when it was not refused.
fn refusal(result: &Value) -> String {
    if !tool_failed(result) {
        return "ok".into();
    }
    tool_text(result)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

async fn pty_state(c: &RpcClient, ws: &str, terminal: &str) -> Option<String> {
    c.list_pty_agents(Some(ws.to_string()))
        .await
        .expect("list_pty_agents")
        .into_iter()
        .find(|a| a.terminal_id == terminal)
        .map(|a| a.state.as_str().to_string())
}

async fn reaches(c: &RpcClient, ws: &str, terminal: &str, want: &str) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        if pty_state(c, ws, terminal).await.as_deref() == Some(want) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}
