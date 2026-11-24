//! A PTY agent cut off mid-turn by a server restart comes back in its own tab: the next server
//! relaunches it under the same terminal id on `--resume` of its own conversation, and nudges it
//! to carry on. An idle one stays exited, listed as stopped for its spawner to resume.
//!
//! The harness is `agent_stub`'s PTY claude; nothing here covers the real binary.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::claude::memory::project_key;
use hyprlayer_core::models::PtyAgentState;
use hyprlayer_e2e::agent_stub::{
    appears, install, launch_log, launch_pty, lines, set_engine, type_line, PTY_CLAUDE,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::RpcClient;

const IO: Duration = Duration::from_secs(30);
/// The resume waits for a client's environment, which `boot` sends at once.
const RESUME_WAIT: Duration = Duration::from_secs(60);
const NUDGE: &str = "The hyprlayer server restarted and stopped this agent mid-turn.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_restart_resume() {
    let sandbox = Sandbox::new();
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("pty_restart_resume", &sandbox);
    let (server, client) = boot(&mut j, &sandbox).await;
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Resume").await;
    let ws = text(&view, "/item/id");
    let worktree = text(&view, "/item/worktreePath");
    set_engine(&mut j, &client, "claude-code", &pty_claude, &[]).await;

    j.step("one agent is mid-turn and one idle when the server stops");
    let busy = launch(&mut j, &sandbox, &client, &ws, &worktree, "$BUSY").await;
    let idle = launch(&mut j, &sandbox, &client, &ws, &worktree, "$IDLE").await;
    type_line(&client, &busy.terminal, "hold").await;
    let working = eventually(IO, || {
        lines(&launch_log(&sandbox, &busy.terminal).join("hooks.log"))
            .iter()
            .any(|l| l == "UserPromptSubmit exit=0")
    })
    .await
        && wait_state(&client, &busy.terminal, PtyAgentState::Working).await;
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the busy agent reports working, then the server stops cleanly",
        working && stopped.status.success(),
        json!({ "working": working, "exit": stopped.status.code() }),
    );

    j.step("the next server resumes the busy agent in place");
    let (server, client) = boot(&mut j, &sandbox).await;
    let back = wait_listed(&client, &busy.terminal).await;
    let argv = lines(&launch_log(&sandbox, &busy.terminal).join("argv"));
    let resumed_with = argv
        .windows(2)
        .any(|w| w[0] == "--resume" && w[1] == busy.session);
    j.guard(
        "the busy agent is live again under its own terminal id, on --resume of its conversation",
        back && resumed_with,
        json!({ "live": back, "resumeArgs": tail(&argv, 2) }),
    );
    let stdin = launch_log(&sandbox, &busy.terminal).join("stdin.log");
    let nudged = eventually(IO, || lines(&stdin).iter().any(|l| l.contains(NUDGE))).await;
    j.guard(
        "it is told the server restarted and to carry on",
        nudged,
        json!({ "nudged": nudged }),
    );

    j.step("the idle agent stays exited, listed as stopped");
    let live: Vec<String> = client
        .list_pty_agents(None)
        .await
        .expect("list_pty_agents")
        .into_iter()
        .map(|a| a.terminal_id)
        .collect();
    let stopped = client
        .list_stopped_pty_agents()
        .await
        .expect("list_stopped_pty_agents");
    let idle_stopped = stopped
        .iter()
        .any(|a| a.terminal_id == idle.terminal && a.state == PtyAgentState::Exited);
    let idle_argv = lines(&launch_log(&sandbox, &idle.terminal).join("argv"));
    j.guard(
        "the idle agent was not relaunched, and is listed as stopped with its conversation",
        !live.contains(&idle.terminal)
            && idle_stopped
            && !idle_argv.iter().any(|a| a == "--resume"),
        json!({
            "idleLive": live.contains(&idle.terminal),
            "idleStopped": idle_stopped,
            "busyStopped": stopped.iter().any(|a| a.terminal_id == busy.terminal),
        }),
    );

    j.step("closing the idle agent's tab drops it for good");
    client
        .terminal_close(idle.terminal.clone())
        .await
        .expect("terminal_close");
    let gone = !client
        .list_stopped_pty_agents()
        .await
        .expect("list_stopped_pty_agents")
        .iter()
        .any(|a| a.terminal_id == idle.terminal);
    j.guard(
        "a closed tab's agent is no longer listed as stopped",
        gone,
        json!({ "gone": gone }),
    );

    j.step("a second restart before the nudged turn ends resumes it once more, not twice");
    let stopped = server.shutdown(&client).await;
    let (server, client) = boot(&mut j, &sandbox).await;
    let again = wait_listed(&client, &busy.terminal).await;
    let running = holders(&busy.session);
    j.guard(
        "the agent is back again under the same id, with one process",
        stopped.status.success() && again && running == 1,
        json!({ "live": again, "processes": running }),
    );
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the server stops cleanly",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );
    j.finish();
}

struct Launched {
    terminal: String,
    session: String,
}

/// A fresh desktop launch, its conversation written where `claude --resume` finds it.
async fn launch(
    j: &mut Journey,
    sandbox: &Sandbox,
    client: &RpcClient,
    ws: &str,
    worktree: &str,
    alias: &str,
) -> Launched {
    let (launched, _events) = launch_pty(client, ws, "claude-code").await;
    let terminal = launched.terminal_id.clone();
    let session = launched.provider_session_id.clone().unwrap_or_default();
    j.alias(&terminal, alias);
    j.alias(&session, format!("{alias}_SESSION"));
    let transcripts = sandbox
        .home
        .join(".claude")
        .join("projects")
        .join(project_key(worktree));
    std::fs::create_dir_all(&transcripts).expect("create the transcript dir");
    std::fs::write(transcripts.join(format!("{session}.jsonl")), "{}\n")
        .expect("write a transcript");
    appears(&launch_log(sandbox, &terminal).join("hooks.log")).await;
    wait_state(client, &terminal, PtyAgentState::Idle).await;
    Launched { terminal, session }
}

async fn boot(j: &mut Journey, sandbox: &Sandbox) -> (Server, Arc<RpcClient>) {
    let server = Server::start(sandbox).await;
    let client = server.client();
    j.call(
        &client,
        Request::SetEnvOverlay {
            vars: BTreeMap::from([("HYPRLAYER_E2E".to_string(), "1".to_string())]),
        },
    )
    .await
    .expect("set_env_overlay");
    (server, client)
}

async fn wait_state(client: &RpcClient, terminal: &str, state: PtyAgentState) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        if let Ok(list) = client.list_pty_agents(None).await {
            if list
                .iter()
                .any(|a| a.terminal_id == terminal && a.state == state)
            {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Live again: listed, and past its launch.
async fn wait_listed(client: &RpcClient, terminal: &str) -> bool {
    let deadline = tokio::time::Instant::now() + RESUME_WAIT;
    while tokio::time::Instant::now() < deadline {
        if let Ok(list) = client.list_pty_agents(None).await {
            if list.iter().any(|a| {
                a.terminal_id == terminal
                    && !matches!(a.state, PtyAgentState::Exited | PtyAgentState::Starting)
            }) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Running processes resuming the conversation, by their argv.
fn holders(session: &str) -> usize {
    std::process::Command::new("pgrep")
        .args(["-f", "--", &format!("--resume {session}")])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().count())
        .unwrap_or(0)
}

fn tail(argv: &[String], n: usize) -> Vec<String> {
    argv[argv.len().saturating_sub(n)..].to_vec()
}
