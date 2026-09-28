//! An agent session against a staged harness: the turn reaches the provider,
//! its reply is streamed and persisted, a permission ask round-trips, and the
//! session outlives the client that started it the way a terminal does.
//!
//! The harness is a stub speaking claude's stream-json, not the real binary, so
//! nothing here covers the provider's own behaviour — only the contract the
//! driver holds it to.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_server::client::EventStream;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Replies to each user turn with a numbered marker, and asks for permission
/// once the turn text carries `ASK_PERMISSION`. Every line it is handed is
/// appended to `<sandbox>/harness.log` — reached from `$0` because the server's
/// environment is the profile's, with nothing this scenario can add to it — so
/// a scenario can assert what the provider actually received rather than what
/// the client believes it sent.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
log="$(dirname "$0")/../harness.log"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *ASK_QUESTION*)
      printf '{"type":"control_request","request_id":"q-%s","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Ship it?","header":"Ship","multiSelect":false,"options":[{"label":"Yes","description":"ship now"},{"label":"No","description":"hold"}]}]}}}\n' "$turn"
      IFS= read -r response || break
      printf '%s\n' "$response" >> "$log"
      ;;
    *ASK_PERMISSION*)
      printf '{"type":"control_request","request_id":"req-%s","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}\n' "$turn"
      IFS= read -r response || break
      printf '%s\n' "$response" >> "$log"
      ;;
  esac
  printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"E2E_PARTIAL_%s"}},"session_id":"%s","parent_tool_use_id":null}\n' "$turn" "$sid"
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7},"modelUsage":{"claude-e2e":{"contextWindow":200000}}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_lifecycle() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let harness_log = sandbox.root.join("harness.log");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("session_lifecycle", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Session work").await;
    let ws = text(&view, "/item/id");

    j.step("launch a stage session");
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
    let first = drain_turn(&mut events).await;
    j.guard(
        "the harness announces its session",
        first.iter().any(|e| matches!(e, SessionEvent::Init { .. })),
        json!({ "init": first.iter().filter(|e| matches!(e, SessionEvent::Init { .. })).count() }),
    );
    j.guard(
        "the assistant's reply reaches the client",
        assistant_text(&first).contains("E2E_REPLY_1"),
        json!({ "text": assistant_text(&first) }),
    );
    j.guard(
        "the turn completes",
        first
            .iter()
            .any(|e| matches!(e, SessionEvent::TurnDone { .. })),
        json!({ "turn_done": first.iter().filter(|e| matches!(e, SessionEvent::TurnDone { .. })).count() }),
    );
    j.guard(
        "streamed deltas arrive before the finished message",
        first
            .iter()
            .any(|e| matches!(e, SessionEvent::PartialText { text, .. } if text.contains("E2E_PARTIAL_1"))),
        json!({ "partial": first.iter().filter(|e| matches!(e, SessionEvent::PartialText { .. })).count() }),
    );
    j.guard(
        "the result's token accounting reaches the client",
        first
            .iter()
            .any(|e| matches!(e, SessionEvent::ContextWindow { tokens } if *tokens == 200_000)),
        json!({ "context_window": first.iter().find_map(|e| match e {
            SessionEvent::ContextWindow { tokens } => Some(*tokens),
            _ => None,
        }) }),
    );
    let received = std::fs::read_to_string(&harness_log).unwrap_or_default();
    j.guard(
        "the prompt reaches the provider, not just the log",
        received.contains("E2E_PROMPT_ONE"),
        json!({ "provider_saw_prompt": received.contains("E2E_PROMPT_ONE") }),
    );

    let lifecycle = events_until(&mut events_rx, IO, |e| {
        matches!(e, RuntimeEvent::SessionLifecycle { session_id, .. } if *session_id == session.id)
    })
    .await;
    j.guard(
        "launching announces the session's lifecycle state",
        lifecycle
            .iter()
            .any(|e| matches!(e, RuntimeEvent::SessionLifecycle { session_id, .. } if *session_id == session.id)),
        json!({ "states": lifecycle.iter().filter_map(|e| match e {
            RuntimeEvent::SessionLifecycle { state, .. } => Some(format!("{state:?}")),
            _ => None,
        }).collect::<Vec<_>>() }),
    );

    j.step("the turn is persisted");
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let kinds = kinds_of(&log);
    j.guard(
        "the stored log carries the user turn and the reply",
        kinds.iter().any(|k| k == "userMessage") && kinds.iter().any(|k| k == "assistantText"),
        json!({ "kinds": kinds }),
    );
    let sessions = j
        .call(
            &client,
            Request::ListSessions {
                workstream_id: ws.clone(),
            },
        )
        .await
        .expect("list_sessions");
    j.guard(
        "the workstream lists its session",
        session_ids(&sessions).contains(&session.id),
        json!({ "sessions": session_ids(&sessions).len() }),
    );

    j.step("answer a permission ask");
    j.call(
        &client,
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
    let asked = recv_until(&mut events, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    let request_id = asked.iter().find_map(|e| match e {
        SessionEvent::PermissionAsk { request_id, .. } => Some(request_id.clone()),
        _ => None,
    });
    j.guard(
        "the provider's permission request reaches the client",
        request_id.is_some(),
        json!({ "asked": request_id.is_some() }),
    );
    j.call(
        &client,
        Request::RespondPermission {
            session_id: session.id.clone(),
            request_id: request_id.clone().unwrap_or_default(),
            allow: true,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
    let after = drain_turn(&mut events).await;
    j.guard(
        "the turn continues once permission is granted",
        assistant_text(&after).contains("E2E_REPLY_2"),
        json!({ "text": assistant_text(&after) }),
    );

    j.step("answer a structured question");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "ASK_QUESTION please".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
        },
    )
    .await
    .expect("send_user_message");
    let asked = recv_until(&mut events, |e| {
        matches!(e, SessionEvent::QuestionAsk { .. })
    })
    .await;
    let question = asked.iter().find_map(|e| match e {
        SessionEvent::QuestionAsk {
            request_id,
            questions,
        } => Some((request_id.clone(), questions.clone())),
        _ => None,
    });
    j.guard(
        "an AskUserQuestion tool call arrives as a question, not a permission",
        question
            .as_ref()
            .is_some_and(|(_, qs)| qs.first().is_some_and(|q| q.header == "Ship")),
        json!({ "headers": question.as_ref().map(|(_, qs)| qs.iter().map(|q| q.header.clone()).collect::<Vec<_>>()) }),
    );
    j.guard(
        "its options survive the round trip",
        question.as_ref().is_some_and(|(_, qs)| {
            qs.first()
                .is_some_and(|q| q.options.iter().any(|o| o.label == "Yes"))
        }),
        json!({ "options": question.as_ref().and_then(|(_, qs)| qs.first()).map(|q| q.options.iter().map(|o| o.label.clone()).collect::<Vec<_>>()) }),
    );
    let (question_id, questions) = question.expect("a question");
    let mut answers = std::collections::BTreeMap::new();
    answers.insert(questions[0].header.clone(), "Yes".to_string());
    j.call(
        &client,
        Request::AnswerQuestion {
            session_id: session.id.clone(),
            request_id: question_id,
            answers: hyprlayer_core::models::session::QuestionAnswers(answers),
        },
    )
    .await
    .expect("answer_question");
    let answered = drain_turn(&mut events).await;
    j.guard(
        "the turn continues once the question is answered",
        assistant_text(&answered).contains("E2E_REPLY_3"),
        json!({ "text": assistant_text(&answered) }),
    );
    let sent = std::fs::read_to_string(&harness_log).unwrap_or_default();
    j.guard(
        "the chosen option reaches the provider",
        sent.contains("Yes"),
        json!({ "provider_saw_answer": sent.contains("Yes") }),
    );

    j.step("adjust the live session");
    for (label, request) in [
        (
            "model",
            Request::SetSessionModel {
                session_id: session.id.clone(),
                model: Some("claude-e2e".into()),
            },
        ),
        (
            "effort",
            Request::SetSessionEffort {
                session_id: session.id.clone(),
                effort: Some("high".into()),
            },
        ),
    ] {
        let out = j.call(&client, request).await;
        let evidence = match &out {
            Ok(_) => json!({ "accepted": true }),
            Err(e) => json!({ "code": e.code }),
        };
        j.guard(
            &format!("a live session takes a new {label}"),
            out.is_ok(),
            evidence,
        );
    }
    // Interrupt ends the turn, not the session: `require_live` would fail after
    // a kill, so a session that still accepts work is the property.
    let interrupted = j
        .call(
            &client,
            Request::InterruptSession {
                session_id: session.id.clone(),
            },
        )
        .await;
    let evidence = match &interrupted {
        Ok(_) => json!({ "accepted": true }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "an idle session accepts an interrupt",
        interrupted.is_ok(),
        evidence,
    );
    let still_active = j
        .call(&client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    j.guard(
        "interrupting does not end the session",
        active_ids(&still_active).contains(&session.id),
        json!({ "active": active_ids(&still_active).len() }),
    );

    j.step("the session outlives its client");
    drop(events);
    let active = j
        .call(&client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    j.guard(
        "a dropped stream does not end the session",
        active_ids(&active).contains(&session.id),
        json!({ "active": active_ids(&active).len() }),
    );
    let reattached = client.attach_session_stream(session.id.clone()).await;
    let evidence = match &reattached {
        Ok(_) => json!({ "attached": true }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "the session can be reattached",
        reattached.is_ok(),
        evidence,
    );
    let mut events = reattached.expect("a live session").1;
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "E2E_PROMPT_THREE".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
        },
    )
    .await
    .expect("send_user_message");
    // A reattach replays the log before it streams anything live, so the next
    // `TurnDone` is the first turn's, not this one's.
    let third = recv_until(
        &mut events,
        |e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_4")),
    )
    .await;
    j.guard(
        "the reattached session still takes turns",
        assistant_text(&third).contains("E2E_REPLY_4"),
        json!({ "text": assistant_text(&third) }),
    );

    j.step("kill the session");
    j.call(
        &client,
        Request::KillSession {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("kill_session");
    let ended = recv_until(&mut events, |e| matches!(e, SessionEvent::Exited { .. })).await;
    j.guard(
        "killing the session reports the harness exit",
        ended
            .iter()
            .any(|e| matches!(e, SessionEvent::Exited { .. })),
        json!({ "exited": ended.iter().filter(|e| matches!(e, SessionEvent::Exited { .. })).count() }),
    );
    let active = j
        .call(&client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    j.guard(
        "a killed session is no longer active",
        !active_ids(&active).contains(&session.id),
        json!({ "active": active_ids(&active).len() }),
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

/// Everything up to and including the turn's `TurnDone`.
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

fn kinds_of(log: &Value) -> Vec<String> {
    log.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.pointer("/event/kind").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn session_ids(sessions: &Value) -> Vec<String> {
    ids(sessions, "/id")
}

fn active_ids(sessions: &Value) -> Vec<String> {
    ids(sessions, "/session/id")
}

fn ids(value: &Value, pointer: &str) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(|i| text(i, pointer)).collect())
        .unwrap_or_default()
}
