//! A session keeps the model and effort it runs on across a relaunch that names
//! neither: a resume after a server restart, after a switch live or while exited,
//! and a permission-mode restart. A restart that names a model sets both, and is
//! how a session stored on a retired model recovers.
#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use hyprlayer_core::{Harness, PermissionMode, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const PICKED: &str = "claude-e2e-picked";
const SWITCHED: &str = "claude-e2e-switched";
const OTHER: &str = "claude-e2e-other";
const EXITED_PICK: &str = "claude-e2e-exited-pick";
const RETIRED: &str = "claude-e2e-retired";

/// Reports the `--model` it was launched with in `init`, as claude does, and
/// appends each launch's argv as one line to `resume-control/<session>.argv`.
/// On `claude-e2e-retired` every turn fails as claude 2.1.329's did on a retired id, and it stays up.
const ECHO_CLAUDE: &str = r#"#!/bin/sh
argv=$(printf '%s' "$*" | tr '\n' ' ')
sid=""; model="claude-e2e-default"; oneshot=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id|--resume) sid="$2"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    --output-format) [ "$2" = json ] && oneshot=1; shift 2 ;;
    *) shift ;;
  esac
done
if [ -n "$oneshot" ]; then
  cat >/dev/null
  printf '{"type":"result","subtype":"success","is_error":false,"result":"Resume E2E"}\n'
  exit 0
fi
base="$(dirname "$0")/../resume-control"
mkdir -p "$base"
printf '%s\n' "$argv" >> "$base/$sid.argv"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"%s","tools":[]}\n' "$sid" "$model"
turn=0
gone="There's an issue with the selected model ($model). It may not exist or you may not have access to it. Run --model to pick a different model."
while IFS= read -r line; do
  case "$line" in *'"type":"user"'*) ;; *) continue ;; esac
  turn=$((turn + 1))
  if [ "$model" = claude-e2e-retired ]; then
    printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"%s"}]},"session_id":"%s","error":"model_not_found","api_error_status":404}\n' "$gone" "$sid"
    printf '{"type":"result","subtype":"success","is_error":true,"result":"%s","session_id":"%s","api_error_status":404}\n' "$gone" "$sid"
    continue
  fi
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_keeps_model() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", ECHO_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("resume_keeps_model", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Resume model").await;
    let ws = text(&view, "/item/id");

    j.step("start a session on a picked model and effort");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "hello".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                model: Some(PICKED.into()),
                effort: Some("high".into()),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    let id = session.id.clone();
    j.alias(&id, "$S");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    turn_done(&mut events).await;
    let argv = sandbox
        .root
        .join("resume-control")
        .join(format!("{id}.argv"));
    let first = launch(&argv);
    j.guard(
        "the first launch runs the picked model and effort",
        first["model"] == PICKED && first["effort"] == "high",
        first,
    );

    j.step("a resume after a server restart keeps them");
    drop(events);
    let stopped = server.shutdown(&client).await;
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events = resume(&client, &id).await;
    send(&mut j, &client, &id, "after the restart").await;
    let replied = turn_done(&mut events).await;
    let resumed = launch(&argv);
    j.guard(
        "the relaunch resumes the conversation on the picked model and effort",
        stopped.status.success()
            && replied
            && resumed["resumed"] == true
            && resumed["model"] == PICKED
            && resumed["effort"] == "high",
        json!({ "exit": stopped.status.code(), "replied": replied, "launch": resumed }),
    );
    let listed = listed_model(&mut j, &client, &ws, &id).await;
    j.guard(
        "list_sessions still shows the picked model",
        listed == PICKED,
        json!({ "model": listed }),
    );

    j.step("a model and effort switched in place survive the next resume");
    for request in [
        Request::SetSessionModel {
            session_id: id.clone(),
            model: Some(SWITCHED.into()),
        },
        Request::SetSessionEffort {
            session_id: id.clone(),
            effort: Some("low".into()),
        },
    ] {
        j.call(&client, request).await.expect("switch in place");
    }
    j.call(
        &client,
        Request::KillSession {
            session_id: id.clone(),
        },
    )
    .await
    .expect("kill_session");
    recv_until(&mut events, |e| matches!(e, SessionEvent::Exited { .. })).await;
    let mut events = resume(&client, &id).await;
    send(&mut j, &client, &id, "after the switch").await;
    let replied = turn_done(&mut events).await;
    let switched = launch(&argv);
    j.guard(
        "the resume runs the switched model and effort",
        replied && switched["model"] == SWITCHED && switched["effort"] == "low",
        json!({ "replied": replied, "launch": switched }),
    );
    let listed = listed_model(&mut j, &client, &ws, &id).await;
    j.guard(
        "list_sessions shows the switched model",
        listed == SWITCHED,
        json!({ "model": listed }),
    );

    j.step("a permission-mode restart keeps them");
    drop(events);
    let (_, _, _, mut events) = client
        .restart_session_stream(id.clone(), None, None, Some(PermissionMode::AcceptEdits))
        .await
        .expect("restart_session");
    send(&mut j, &client, &id, "after the mode change").await;
    let replied = turn_done(&mut events).await;
    let restarted = launch(&argv);
    j.guard(
        "the restart names no model and runs the session's",
        replied && restarted["model"] == SWITCHED && restarted["effort"] == "low",
        json!({ "replied": replied, "launch": restarted }),
    );

    j.step("a restart that names a model sets both");
    drop(events);
    let (_, _, _, mut events) = client
        .restart_session_stream(id.clone(), Some(OTHER.into()), None, None)
        .await
        .expect("restart_session");
    send(&mut j, &client, &id, "after the model change").await;
    let replied = turn_done(&mut events).await;
    let named = launch(&argv);
    j.guard(
        "the named model runs with no effort",
        replied && named["model"] == OTHER && named["effort"].is_null(),
        json!({ "replied": replied, "launch": named }),
    );

    j.step("a model and effort picked while the session is exited persist");
    j.call(
        &client,
        Request::KillSession {
            session_id: id.clone(),
        },
    )
    .await
    .expect("kill_session");
    recv_until(&mut events, |e| matches!(e, SessionEvent::Exited { .. })).await;
    pick(&mut j, &client, &id, EXITED_PICK, "medium").await;
    let mut events = resume(&client, &id).await;
    send(&mut j, &client, &id, "after the exited pick").await;
    let replied = turn_done(&mut events).await;
    let picked = launch(&argv);
    j.guard(
        "the resume runs the model and effort picked while exited",
        replied && picked["model"] == EXITED_PICK && picked["effort"] == "medium",
        json!({ "replied": replied, "launch": picked }),
    );

    j.step("a session stored on a retired model fails visibly and a pick recovers it");
    j.call(
        &client,
        Request::KillSession {
            session_id: id.clone(),
        },
    )
    .await
    .expect("kill_session");
    recv_until(&mut events, |e| matches!(e, SessionEvent::Exited { .. })).await;
    pick(&mut j, &client, &id, RETIRED, "medium").await;
    let mut events = resume(&client, &id).await;
    send(&mut j, &client, &id, "on the retired model").await;
    let turn = recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    let said = turn.iter().any(|e| {
        matches!(e, SessionEvent::AssistantText { text, .. } if text.contains(&format!("issue with the selected model ({RETIRED})")))
    });
    let failed = turn.iter().any(|e| {
        matches!(e, SessionEvent::TurnDone { turn_status, .. } if turn_status.as_deref() == Some("failed"))
    });
    let active = j
        .call(&client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    let live = active
        .as_array()
        .is_some_and(|all| all.iter().any(|a| a["session"]["id"] == id.as_str()));
    j.guard(
        "the turn fails with claude's model error, and the session stays live",
        said && failed && live,
        json!({ "errorShown": said, "turnFailed": failed, "live": live }),
    );
    drop(events);
    let (_, _, _, mut events) = client
        .restart_session_stream(id.clone(), Some(OTHER.into()), None, None)
        .await
        .expect("restart_session");
    send(&mut j, &client, &id, "after picking a model").await;
    let replied = turn_done(&mut events).await;
    let recovered = launch(&argv);
    j.guard(
        "picking a model restarts the live session on it",
        replied && recovered["model"] == OTHER,
        json!({ "replied": replied, "launch": recovered }),
    );

    j.step("shutdown");
    drop(events);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

/// The latest launch's resume, model and effort flags.
fn launch(argv: &Path) -> Value {
    let log = std::fs::read_to_string(argv).unwrap_or_default();
    let last = log.lines().last().unwrap_or_default();
    let words: Vec<&str> = last.split_whitespace().collect();
    let flag = |name: &str| {
        words
            .iter()
            .position(|w| *w == name)
            .and_then(|i| words.get(i + 1))
            .map(|v| json!(v))
            .unwrap_or(Value::Null)
    };
    json!({
        "launches": log.lines().count(),
        "resumed": words.contains(&"--resume"),
        "model": flag("--model"),
        "effort": flag("--effort"),
    })
}

/// What the desktop sends for a pick on a tab with no live harness.
async fn pick(j: &mut Journey, client: &RpcClient, id: &str, model: &str, effort: &str) {
    for request in [
        Request::SetSessionModel {
            session_id: id.to_string(),
            model: Some(model.into()),
        },
        Request::SetSessionEffort {
            session_id: id.to_string(),
            effort: Some(effort.into()),
        },
    ] {
        j.call(client, request).await.expect("pick");
    }
}

async fn resume(client: &RpcClient, id: &str) -> EventStream<SessionEvent> {
    let (_, _, _, events) = client
        .resume_session_stream(id.to_string(), None, None, None, None)
        .await
        .expect("resume_session");
    events
}

async fn send(j: &mut Journey, client: &RpcClient, id: &str, text: &str) {
    j.call(
        client,
        Request::SendUserMessage {
            session_id: id.to_string(),
            text: text.into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
}

async fn listed_model(j: &mut Journey, client: &RpcClient, ws: &str, id: &str) -> Value {
    let sessions = j
        .call(
            client,
            Request::ListSessions {
                workstream_id: ws.to_string(),
            },
        )
        .await
        .expect("list_sessions");
    sessions
        .as_array()
        .and_then(|all| all.iter().find(|s| s["id"] == id))
        .map(|s| s["model"].clone())
        .unwrap_or(Value::Null)
}

async fn turn_done(events: &mut EventStream<SessionEvent>) -> bool {
    recv_until(events, |e| matches!(e, SessionEvent::TurnDone { .. }))
        .await
        .iter()
        .any(|e| matches!(e, SessionEvent::TurnDone { .. }))
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
