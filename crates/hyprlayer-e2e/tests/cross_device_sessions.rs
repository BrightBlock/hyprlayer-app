//! Two clients, one session: opening a session another device started never
//! kills its turn in progress, and the resume reports the mode actually
//! running so the second client's tab can adopt it.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, LogCursor, PermissionMode, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::EventStream;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Merges `plan_mode.rs`'s spawn log with `session_lifecycle.rs`'s
/// `ASK_PERMISSION` hold, so a turn can be parked mid-flight while a second
/// client attaches.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""; mode=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id|--resume) sid="$2"; shift 2 ;;
    --permission-mode) mode="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$mode" ] && printf 'spawn %s\n' "$mode" >> "$(dirname "$0")/../spawns.log"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","permissionMode":"%s","tools":[]}\n' "$sid" "$mode"
turn=0
while IFS= read -r line; do
  case "$line" in *control_response*) continue ;; esac
  turn=$((turn + 1))
  case "$line" in
    *ASK_PERMISSION*)
      printf '{"type":"control_request","request_id":"req-%s","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}\n' "$turn"
      IFS= read -r response || break ;;
  esac
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_device_sessions() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let spawns_log = sandbox.root.join("spawns.log");
    let spawns = || -> Vec<String> {
        std::fs::read_to_string(&spawns_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    };
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("cross_device_sessions", &sandbox);
    let server = Server::start(&sandbox).await;
    let a = server.client();
    let b = server.connect().await;

    let project = onboard_and_import(&mut j, &a, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &a, &project, "Cross-device work").await;
    let ws = text(&view, "/item/id");

    j.step("A starts a session");
    let (session, _prompt, _stream_id, mut a_events) = a
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "E2E_PROMPT_ONE".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode: Some(PermissionMode::Default),
                client_message_id: Some("cmid-start".into()),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$SESSION");
    j.call(
        &a,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let first_turn = drain_turn(&mut a_events).await;
    j.guard(
        "the harness is launched once, in the requested mode",
        spawns() == ["spawn default"],
        json!({ "spawns": spawns() }),
    );
    let first_turn_nonce = first_turn.iter().find_map(|e| match e {
        SessionEvent::UserMessage {
            client_message_id, ..
        } => client_message_id.clone(),
        _ => None,
    });
    j.guard(
        "the first turn carries its nonce",
        first_turn_nonce.as_deref() == Some("cmid-start"),
        json!({ "nonce": first_turn_nonce, "kinds": kinds_seen(&first_turn) }),
    );

    j.step("B opens the session mid-turn in another mode");
    j.call(
        &a,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "ASK_PERMISSION please".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
        },
    )
    .await
    .expect("send_user_message");
    let asked = recv_until(&mut a_events, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    let request_id = asked.iter().find_map(|e| match e {
        SessionEvent::PermissionAsk { request_id, .. } => Some(request_id.clone()),
        _ => None,
    });
    j.guard(
        "A's turn is parked on a permission ask",
        request_id.is_some(),
        json!({ "asked": request_id.is_some() }),
    );

    let resumed = b
        .resume_session_stream(
            session.id.clone(),
            None,
            None,
            Some(PermissionMode::AcceptEdits),
            None,
        )
        .await;
    j.guard(
        "the resume is accepted",
        resumed.is_ok(),
        json!({ "resumed": resumed.is_ok() }),
    );
    let running = match &resumed {
        Ok((_, mode, ..)) => *mode,
        Err(_) => None,
    };
    j.guard(
        "it reports the running mode",
        running == Some(PermissionMode::Default),
        json!({ "running": running }),
    );
    let relaunched = eventually(Duration::from_secs(2), || spawns().len() > 1).await;
    j.guard(
        "no second spawn",
        !relaunched && spawns() == ["spawn default"],
        json!({ "spawns": spawns() }),
    );
    let mut b_events = resumed.expect("a resumed session").3;

    j.call(
        &a,
        Request::RespondPermission {
            session_id: session.id.clone(),
            request_id: request_id.unwrap_or_default(),
            allow: true,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
    let a_done = drain_turn(&mut a_events).await;
    j.guard(
        "A's turn completes",
        a_done
            .iter()
            .any(|e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_2"))),
        json!({ "text": assistant_text(&a_done) }),
    );
    let b_seen = recv_until(
        &mut b_events,
        |e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_2")),
    )
    .await;
    j.guard(
        "B watches the same turn",
        b_seen
            .iter()
            .any(|e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_2"))),
        json!({ "text": assistant_text(&b_seen) }),
    );

    j.step("B restores across the log-then-attach gap");
    drop(b_events);
    let log = j
        .call(
            &b,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let last = log.as_array().and_then(|rows| rows.last()).cloned();
    let cursor_seq = last
        .as_ref()
        .and_then(|row| row.pointer("/seq"))
        .and_then(Value::as_i64);
    let cursor_created_at = last
        .as_ref()
        .and_then(|row| row.pointer("/createdAt"))
        .and_then(Value::as_f64);
    j.guard(
        "B folded a log row to resume from",
        cursor_seq.is_some() && cursor_created_at.is_some(),
        json!({ "seq": cursor_seq, "createdAt": cursor_created_at }),
    );
    let (seq, created_at) = (cursor_seq.unwrap_or(-1), cursor_created_at.unwrap_or(0.0));

    j.call(
        &a,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "E2E_PROMPT_GAP".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
        },
    )
    .await
    .expect("send_user_message");
    let a_gap = drain_turn(&mut a_events).await;
    j.guard(
        "A's gap turn is persisted before B re-attaches",
        a_gap
            .iter()
            .any(|e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_3"))),
        json!({ "text": assistant_text(&a_gap) }),
    );

    let refused = b
        .resume_session_stream(
            session.id.clone(),
            None,
            None,
            None,
            Some(LogCursor::After {
                seq,
                created_at: created_at + 1.0,
            }),
        )
        .await;
    j.guard(
        "a cursor whose row was rewritten is refused",
        matches!(&refused, Err(e) if e.code == "session_epoch_advanced"),
        json!({ "code": refused.as_ref().err().map(|e| e.code.clone()) }),
    );

    let resumed_gap = b
        .resume_session_stream(
            session.id.clone(),
            None,
            None,
            None,
            Some(LogCursor::After { seq, created_at }),
        )
        .await
        .expect("resume_session");
    let mut b_gap_events = resumed_gap.3;
    let b_gap = drain_turn(&mut b_gap_events).await;
    j.guard(
        "B's stream carries the events persisted between its log read and its attach",
        b_gap
            .iter()
            .any(|e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_3"))),
        json!({ "kinds": kinds_seen(&b_gap) }),
    );

    j.step("a prompt reaches every viewer");
    j.call(
        &a,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "E2E_PROMPT_FOUR".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: Some("cmid-four".into()),
        },
    )
    .await
    .expect("send_user_message");
    let b_seen_four = recv_until(
        &mut b_gap_events,
        |e| matches!(e, SessionEvent::UserMessage { text, .. } if text.contains("E2E_PROMPT_FOUR")),
    )
    .await;
    let b_nonce = user_message_nonce(&b_seen_four, "E2E_PROMPT_FOUR");
    j.guard(
        "B sees A's prompt live",
        b_nonce.as_deref() == Some("cmid-four"),
        json!({ "nonce": b_nonce }),
    );
    let a_seen_four = recv_until(
        &mut a_events,
        |e| matches!(e, SessionEvent::UserMessage { text, .. } if text.contains("E2E_PROMPT_FOUR")),
    )
    .await;
    let a_nonce = user_message_nonce(&a_seen_four, "E2E_PROMPT_FOUR");
    j.guard(
        "A's echo carries the same nonce",
        a_nonce.as_deref() == Some("cmid-four"),
        json!({ "nonce": a_nonce }),
    );
    let log = j
        .call(
            &a,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let stored_nonce = log.as_array().and_then(|rows| {
        rows.iter()
            .rev()
            .find(|r| r.pointer("/event/kind").and_then(Value::as_str) == Some("userMessage"))
            .and_then(|r| r.pointer("/event/clientMessageId"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    j.guard(
        "the stored row keeps the nonce",
        stored_nonce.as_deref() == Some("cmid-four"),
        json!({ "nonce": stored_nonce }),
    );

    j.step("B sees A's title and first prompt");
    j.call(
        &a,
        Request::SetSessionTitle {
            session_id: session.id.clone(),
            title: Some("Cross-device work".into()),
        },
    )
    .await
    .expect("set_session_title");
    let sessions = j
        .call(
            &b,
            Request::ListSessions {
                workstream_id: ws.clone(),
            },
        )
        .await
        .expect("list_sessions");
    let row = session_row(&sessions, &session.id);
    let title = row
        .as_ref()
        .and_then(|r| r.pointer("/title"))
        .and_then(Value::as_str);
    j.guard(
        "the title crosses devices",
        title == Some("Cross-device work"),
        json!({ "title": title }),
    );
    let preview = row
        .as_ref()
        .and_then(|r| r.pointer("/preview"))
        .and_then(Value::as_str);
    j.guard(
        "the preview is the first prompt",
        preview == Some("E2E_PROMPT_ONE"),
        json!({ "preview": preview }),
    );

    j.call(
        &a,
        Request::SetSessionTitle {
            session_id: session.id.clone(),
            title: Some("   ".into()),
        },
    )
    .await
    .expect("set_session_title");
    let sessions = j
        .call(
            &b,
            Request::ListSessions {
                workstream_id: ws.clone(),
            },
        )
        .await
        .expect("list_sessions");
    let cleared_title =
        session_row(&sessions, &session.id).and_then(|r| r.pointer("/title").cloned());
    j.guard(
        "a blank title clears it",
        cleared_title.as_ref().is_some_and(Value::is_null),
        json!({ "title": cleared_title }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&a).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn drain_turn(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    recv_until(events, |e| matches!(e, SessionEvent::TurnDone { .. })).await
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

fn kinds_seen(events: &[SessionEvent]) -> Vec<&'static str> {
    events.iter().map(SessionEvent::kind).collect()
}

fn assistant_text(events: &[SessionEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::AssistantText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn session_row(sessions: &Value, session_id: &str) -> Option<Value> {
    sessions
        .as_array()?
        .iter()
        .find(|r| r.pointer("/id").and_then(Value::as_str) == Some(session_id))
        .cloned()
}

fn user_message_nonce(events: &[SessionEvent], text_contains: &str) -> Option<String> {
    events.iter().find_map(|e| match e {
        SessionEvent::UserMessage {
            text,
            client_message_id,
            ..
        } if text.contains(text_contains) => client_message_id.clone(),
        _ => None,
    })
}
