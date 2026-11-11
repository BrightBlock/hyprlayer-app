//! A harness whose credentials were rejected: its error text carries an auth flag
//! live and in the stored log, and once signed in a restart resumes the session,
//! unannounced since the server outlived the harness, and the resent message lands.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::EventStream;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Rejects every turn until `<sandbox>/signed-in` exists, with the lines claude
/// 2.1.322 sends for an expired login. A `SUBAGENT` turn fails inside a subagent
/// instead. Each spawn is logged to `<sandbox>/spawns.log` as `new|resume <id>`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
how="new"
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    --resume) sid="$2"; how="resume"; shift 2 ;;
    *) shift ;;
  esac
done
root="$(dirname "$0")/.."
[ -n "$sid" ] && printf '%s %s\n' "$how" "$sid" >> "$root/spawns.log"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *SUBAGENT*)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Please run /login"}]},"parent_tool_use_id":"toolu_sub","error":"authentication_failed","session_id":"%s"}\n' "$sid"
      ;;
    *)
      if [ ! -e "$root/signed-in" ]; then
        printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Invalid API key \302\267 Please run /login"}]},"parent_tool_use_id":null,"error":"authentication_failed","session_id":"%s"}\n' "$sid"
        printf '{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key \302\267 Please run /login","session_id":"%s","num_turns":1,"total_cost_usd":0}\n' "$sid"
        continue
      fi
      ;;
  esac
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_required() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let spawns_log = sandbox.root.join("spawns.log");
    let spawns = || -> Vec<String> {
        std::fs::read_to_string(&spawns_log)
            .unwrap_or_default()
            .lines()
            .map(|l| l.split(' ').next().unwrap_or_default().to_string())
            .collect()
    };
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("auth_required", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Auth work").await;
    let ws = text(&view, "/item/id");

    j.step("the first message hits an expired login");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "E2E_PROMPT_ONE".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$SESSION");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let failed = drain_turn(&mut events).await;
    let flagged = auth_texts(&failed);
    j.guard(
        "the client hears claude's error text flagged as an auth failure",
        flagged == ["Invalid API key · Please run /login"],
        json!({ "flagged": flagged }),
    );

    j.step("the flag is history, so a reload or another device sees it");
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let stored = stored_auth_texts(&log);
    j.guard(
        "the stored error text keeps its auth flag",
        stored == ["Invalid API key · Please run /login"],
        json!({ "flagged": stored }),
    );

    j.step("a subagent's rejected login raises no second prompt");
    send(&mut j, &client, &session.id, "SUBAGENT check").await;
    let sub = drain_turn(&mut events).await;
    j.guard(
        "a subagent's auth error is not the session's",
        auth_texts(&sub).is_empty()
            && sub.iter().any(
                |e| matches!(e, SessionEvent::AssistantText { text, .. } if text == "Please run /login"),
            ),
        json!({ "flagged": auth_texts(&sub) }),
    );
    drop(events);

    j.step("sign in, restart, and resend the failed message");
    std::fs::write(sandbox.root.join("signed-in"), "").expect("sign in");
    let restarted = client
        .restart_session_stream(session.id.clone(), None, None, None)
        .await;
    j.guard(
        "the restart is accepted for a session whose only message failed",
        restarted.is_ok(),
        json!({ "restarted": restarted.as_ref().err().map(|e| e.message.clone()) }),
    );
    let mut events = restarted.expect("a restarted session").3;
    send(&mut j, &client, &session.id, "E2E_PROMPT_ONE").await;
    let retried = drain_turn(&mut events).await;
    j.guard(
        "the relaunched harness resumes the same conversation",
        spawns() == ["new", "resume"],
        json!({ "spawns": spawns() }),
    );
    j.guard(
        "the resent message is answered with no new prompt",
        retried.iter().any(
            |e| matches!(e, SessionEvent::AssistantText { text, .. } if text.starts_with("E2E_REPLY_")),
        ) && auth_texts(&retried).is_empty(),
        json!({ "kinds": retried.iter().map(SessionEvent::kind).collect::<Vec<_>>() }),
    );
    drop(events);
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let boundaries: Vec<Value> = log
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r.pointer("/event/kind").and_then(Value::as_str) == Some("relaunched"))
        .map(|r| r.pointer("/event/quiet").cloned().unwrap_or(Value::Null))
        .collect();
    j.guard(
        "a restart the server outlived is not announced",
        boundaries == [json!(true)],
        json!({ "quiet": boundaries }),
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

async fn send(
    j: &mut Journey,
    client: &hyprlayer_server::client::RpcClient,
    session_id: &str,
    text: &str,
) {
    j.call(
        client,
        Request::SendUserMessage {
            session_id: session_id.to_string(),
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

async fn drain_turn(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            let done = matches!(event, SessionEvent::TurnDone { .. });
            seen.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    seen
}

fn auth_texts(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::AssistantText {
                text,
                auth_required: true,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn stored_auth_texts(log: &Value) -> Vec<String> {
    log.as_array()
        .into_iter()
        .flatten()
        .filter(|r| r.pointer("/event/authRequired") == Some(&Value::Bool(true)))
        .filter_map(|r| r.pointer("/event/text").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}
