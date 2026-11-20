//! Archiving a workstream ends its live session quietly: the row reads idle with
//! an end time, the log gains no exit, and a restore resumes the same conversation.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Completes every turn at once; each launch's arguments land in `<sandbox>/harness-args.log`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$(dirname "$0")/../harness-args.log"
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id|--resume) sid="$2"; shift 2 ;;
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
  printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":%s,"total_cost_usd":0,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_ends_sessions() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let args_log = sandbox.root.join("harness-args.log");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("archive_ends_sessions", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Archive live").await;
    let ws = text(&view, "/item/id");

    j.step("run a session to the end of its turn");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "hello".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    let sid = session.id.clone();
    j.alias(&sid, "$SESSION");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: sid.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let turn = recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    let row = session_row(&mut j, &client, &ws, &sid).await;
    let provider = text(&row, "/claudeSessionId");
    j.alias(&provider, "$PROVIDER");
    let live = is_live(&mut j, &client, &sid).await;
    j.guard(
        "the turn completes and the harness stays up",
        turn.iter()
            .any(|e| matches!(e, SessionEvent::TurnDone { .. }))
            && row["state"] == json!("running")
            && live,
        json!({ "state": row["state"], "live": live }),
    );

    j.step("archive the workstream");
    let before = log_kinds(&mut j, &client, &sid).await;
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: ws.clone(),
            skip_scripts: false,
        },
    )
    .await
    .expect("archive_workstream");
    let exited = recv_until(&mut events, |e| matches!(e, SessionEvent::Exited { .. })).await;
    let outcome = exited.iter().find_map(|e| match e {
        SessionEvent::Exited { outcome, .. } => Some(*outcome),
        _ => None,
    });
    let live = is_live(&mut j, &client, &sid).await;
    j.guard(
        "archive stops the harness",
        outcome.is_some() && !live,
        json!({ "outcome": outcome, "live": live }),
    );
    let row = session_row(&mut j, &client, &ws, &sid).await;
    let ended = !row["endedAt"].is_null();
    j.guard(
        "the row rests idle, with an end time",
        row["state"] == json!("idle") && ended && row["working"] == json!(false),
        json!({ "state": row["state"], "ended": ended, "working": row["working"] }),
    );
    let after = log_kinds(&mut j, &client, &sid).await;
    j.guard(
        "archive adds nothing to the log",
        after == before,
        json!({ "rows": after.len(), "unchanged": after == before }),
    );

    j.step("restore and resume");
    let restored = j
        .call(
            &client,
            Request::RestoreWorkstream {
                id: ws.clone(),
                target: None,
            },
        )
        .await
        .expect("restore_workstream");
    j.guard(
        "the workstream comes back active",
        restored["item"]["status"] == json!("active"),
        json!({ "status": restored["item"]["status"] }),
    );
    // What a restored tab reads: no live harness and a log with no exit, so it shows idle.
    let live = is_live(&mut j, &client, &sid).await;
    let unchanged = log_kinds(&mut j, &client, &sid).await == before;
    j.guard(
        "the restored session rests idle until the next send",
        !live && unchanged,
        json!({ "live": live, "log_unchanged": unchanged }),
    );
    let (_, _, _, mut resumed) = client
        .resume_session_stream(sid.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: sid.clone(),
            text: "after restore".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let turn = recv_until(&mut resumed, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    // Version and model probes run the stub too; only a session launch names its conversation.
    let launches: Vec<String> = std::fs::read_to_string(&args_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("--session-id ") || l.contains("--resume "))
        .map(str::to_string)
        .collect();
    let resumes_own = launches
        .last()
        .is_some_and(|l| l.contains(&format!("--resume {provider}")));
    j.guard(
        "the relaunch resumes the archived conversation",
        launches.len() == 2 && resumes_own,
        json!({ "launches": launches.len(), "resumes_own": resumes_own }),
    );
    let row = session_row(&mut j, &client, &ws, &sid).await;
    let live = is_live(&mut j, &client, &sid).await;
    let ended = !row["endedAt"].is_null();
    j.guard(
        "the resumed session answers and is no longer ended",
        turn.iter()
            .any(|e| matches!(e, SessionEvent::TurnDone { .. }))
            && row["state"] == json!("running")
            && !ended
            && live,
        json!({ "state": row["state"], "ended": ended, "live": live }),
    );

    j.step("shutdown");
    drop(resumed);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn session_row(j: &mut Journey, client: &RpcClient, ws: &str, id: &str) -> Value {
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
        .and_then(|rows| rows.iter().find(|r| text(r, "/id") == id))
        .cloned()
        .unwrap_or(Value::Null)
}

async fn log_kinds(j: &mut Journey, client: &RpcClient, id: &str) -> Vec<String> {
    let log = j
        .call(
            client,
            Request::GetSessionLog {
                session_id: id.to_string(),
            },
        )
        .await
        .expect("get_session_log");
    log.as_array()
        .map(|rows| rows.iter().map(|r| text(r, "/kind")).collect())
        .unwrap_or_default()
}

async fn is_live(j: &mut Journey, client: &RpcClient, id: &str) -> bool {
    let active = j
        .call(client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    active
        .as_array()
        .is_some_and(|rows| rows.iter().any(|r| text(r, "/session/id") == id))
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
