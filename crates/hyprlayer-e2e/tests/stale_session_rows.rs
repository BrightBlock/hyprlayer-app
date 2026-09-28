//! A session row outlives its harness process: whether the server stops cleanly
//! or dies, the next server finds no session live and settles the rows the last
//! one left mid-turn, while an ended session keeps its outcome.
#![cfg(unix)]

use std::time::{Duration, Instant};

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Asks for permission on any turn carrying `ASK_PERMISSION`, then waits for
/// the answer; every other turn completes at once.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *ASK_PERMISSION*)
      printf '{"type":"control_request","request_id":"req-%s","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}\n' "$turn"
      IFS= read -r response || break
      ;;
  esac
  printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":%s,"total_cost_usd":0,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_session_rows() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("stale_session_rows", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Stale rows").await;
    let ws = text(&view, "/item/id");

    j.step("park one session on a permission ask and kill another");
    let parked = start_session(&mut j, &client, &ws, "ASK_PERMISSION first").await;
    j.alias(&parked.0, "$PARKED");
    let mut parked_events = parked.1;
    let asked = recv_until(&mut parked_events, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    j.guard(
        "the harness asks for permission",
        asked
            .iter()
            .any(|e| matches!(e, SessionEvent::PermissionAsk { .. })),
        json!({ "asked": asked.iter().any(|e| matches!(e, SessionEvent::PermissionAsk { .. })) }),
    );
    let (ended, mut ended_events) = start_session(&mut j, &client, &ws, "hello").await;
    j.alias(&ended, "$ENDED");
    j.call(
        &client,
        Request::KillSession {
            session_id: ended.clone(),
        },
    )
    .await
    .expect("kill_session");
    recv_until(&mut ended_events, |e| {
        matches!(e, SessionEvent::Exited { .. })
    })
    .await;
    let states = session_states(&mut j, &client, &ws).await;
    j.guard(
        "the rows record a parked and an ended session",
        state_of(&states, &parked.0) == "awaiting_permission"
            && state_of(&states, &ended) == "killed",
        states.clone(),
    );

    j.step("restart after a clean shutdown");
    drop(parked_events);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the server stops",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let active = j
        .call(&client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    j.guard(
        "no session is live on a fresh server",
        active.as_array().is_some_and(Vec::is_empty),
        json!({ "active": active.as_array().map(Vec::len) }),
    );
    let states = session_states(&mut j, &client, &ws).await;
    j.guard(
        "the parked row is settled to idle",
        state_of(&states, &parked.0) == "idle",
        states.clone(),
    );
    j.guard(
        "the ended row keeps its outcome",
        state_of(&states, &ended) == "killed",
        states.clone(),
    );

    j.step("resume, park again, and restart after a crash");
    let (_, _, _, mut resumed) = client
        .resume_session_stream(parked.0.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: parked.0.clone(),
            text: "ASK_PERMISSION again".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
        },
    )
    .await
    .expect("send_user_message");
    let asked = recv_until(&mut resumed, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    let states = session_states(&mut j, &client, &ws).await;
    j.guard(
        "the resumed session parks on a new ask",
        asked
            .iter()
            .any(|e| matches!(e, SessionEvent::PermissionAsk { .. }))
            && state_of(&states, &parked.0) == "awaiting_permission",
        states.clone(),
    );
    drop(resumed);
    let socket = server.socket().to_path_buf();
    // kill_on_drop: SIGKILL, so no exit event is ever written.
    drop(server);
    let deadline = Instant::now() + IO;
    let gone = loop {
        if RpcClient::connect_socket(&socket).await.is_err() {
            break true;
        }
        if Instant::now() > deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    j.guard(
        "the crashed server stops listening",
        gone,
        json!({ "gone": gone }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let states = session_states(&mut j, &client, &ws).await;
    j.guard(
        "a crash leaves no row claiming to be live",
        state_of(&states, &parked.0) == "idle" && state_of(&states, &ended) == "killed",
        states.clone(),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn start_session(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    input: &str,
) -> (String, EventStream<SessionEvent>) {
    let (session, _prompt, _stream_id, events) = client
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            input.into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
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

/// `{ id: state }` for every session in the workstream.
async fn session_states(j: &mut Journey, client: &RpcClient, ws: &str) -> Value {
    let sessions = j
        .call(
            client,
            Request::ListSessions {
                workstream_id: ws.to_string(),
            },
        )
        .await
        .expect("list_sessions");
    let map = sessions
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    (
                        text(r, "/id"),
                        r.get("state").cloned().unwrap_or(Value::Null),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Value::Object(map)
}

fn state_of(states: &Value, id: &str) -> String {
    states
        .get(id)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

async fn recv_until(
    events: &mut EventStream<SessionEvent>,
    stop: impl Fn(&SessionEvent) -> bool,
) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            let done = stop(&event);
            seen.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    seen
}
