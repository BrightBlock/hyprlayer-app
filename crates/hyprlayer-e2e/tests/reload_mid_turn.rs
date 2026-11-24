//! A client that attaches while a turn runs, as a reloaded desktop does, learns the turn is
//! open from the attach itself: the log it replays can't say whether its last turn still runs.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Calls Bash, holds the call open until `<sandbox>/release` exists, then finishes the turn.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
gate="$(dirname "$0")/../release"
while IFS= read -r line; do
  printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_sleep","name":"Bash","input":{"command":"sleep 20"}}]},"session_id":"%s","parent_tool_use_id":null}\n' "$sid"
  # Left in place: the title request runs this script too and waits on the same gate.
  while [ ! -f "$gate" ]; do sleep 0.05; done
  printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_sleep","content":"ok"}]},"session_id":"%s","parent_tool_use_id":null}\n' "$sid"
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"slept"}]},"session_id":"%s"}\n' "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"slept","session_id":"%s","num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1}}\n' "$sid"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_mid_turn() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let stub = env!("CARGO_BIN_EXE_opencode_stub");
    sandbox.stage_tool("opencode", &format!("#!/bin/sh\nexec '{stub}' \"$@\"\n"));
    let gate = sandbox.root.join("release");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("reload_mid_turn", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Reload mid turn").await;
    let ws = text(&view, "/item/id");

    j.step("claude: a turn holds a Bash call open");
    let (id, mut first) = start(&mut j, &client, &ws, Harness::Claude, None).await;
    recv_until(&mut first, |seen| {
        seen.iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { .. }))
    })
    .await;
    reattach_mid_turn(&mut j, &client, &id, "claude").await;

    j.step("claude: the call returns and the turn ends");
    std::fs::write(&gate, "").expect("release the call");
    recv_until(&mut first, |seen| seen.iter().any(is_turn_done)).await;
    reattach_after_turn(&mut j, &client, &id, "claude").await;

    j.step("opencode: a run holds its reply");
    std::fs::create_dir_all(sandbox.app_data.join("opencode/opencode-stub"))
        .expect("the stub's dir");
    std::fs::write(
        sandbox.app_data.join("opencode/opencode-stub/control.json"),
        json!({ "reply": 3000 }).to_string(),
    )
    .expect("write the stub's delays");
    let (id, mut first) = start(
        &mut j,
        &client,
        &ws,
        Harness::Opencode,
        Some("opencode/stub-free"),
    )
    .await;
    recv_until(&mut first, |seen| {
        seen.iter()
            .any(|e| matches!(e, SessionEvent::UserMessage { .. }))
    })
    .await;
    reattach_mid_turn(&mut j, &client, &id, "opencode").await;

    j.step("opencode: the reply lands and the turn ends");
    recv_until(&mut first, |seen| seen.iter().any(is_turn_done)).await;
    reattach_after_turn(&mut j, &client, &id, "opencode").await;

    j.finish();
}

async fn start(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    harness: Harness,
    model: Option<&str>,
) -> (String, EventStream<SessionEvent>) {
    let (session, _prompt, _stream_id, events) = client
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_RELOAD".into(),
            vec![],
            StreamStartOpts {
                harness: Some(harness),
                model: model.map(str::to_string),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, format!("${harness:?}").to_uppercase());
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    (session.id, events)
}

async fn reattach_mid_turn(j: &mut Journey, client: &RpcClient, id: &str, harness: &str) {
    let before = hyprlayer_core::store::now_ms();
    let attached = client
        .resume_session_attach(id.to_string(), None, None, None, Some(LogCursor::Empty))
        .await
        .expect("resume_session");
    let started = attached.turn_started_at;
    j.guard(
        &format!("{harness}: an attach mid-turn says the turn is open, and since before it"),
        started.is_some_and(|at| at <= before),
        json!({ "turnOpen": started.is_some() }),
    );
}

async fn reattach_after_turn(j: &mut Journey, client: &RpcClient, id: &str, harness: &str) {
    let attached = client
        .resume_session_attach(id.to_string(), None, None, None, Some(LogCursor::Empty))
        .await
        .expect("resume_session");
    j.guard(
        &format!("{harness}: an attach after the turn ended says none is open"),
        attached.turn_started_at.is_none(),
        json!({ "turnOpen": attached.turn_started_at.is_some() }),
    );
}

fn is_turn_done(e: &SessionEvent) -> bool {
    matches!(e, SessionEvent::TurnDone { .. })
}

async fn recv_until(
    events: &mut EventStream<SessionEvent>,
    stop: impl Fn(&[SessionEvent]) -> bool,
) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            seen.push(event);
            if stop(&seen) {
                return;
            }
        }
    })
    .await;
    seen
}
