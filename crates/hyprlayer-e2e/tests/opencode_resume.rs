//! An OpenCode session resumed after a server restart, or relaunched on another model,
//! continues the same OpenCode session, so the model keeps the earlier turns. When OpenCode
//! no longer has it, the resume starts a new one and says so.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::time::Duration;

use hyprlayer_core::models::{ExitOutcome, QuestionAnswers};
use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;
use tokio::time::Instant;

const IO: Duration = Duration::from_secs(20);
const NONCE: &str = "NONCE-7Q2X";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opencode_resume() {
    let sandbox = Sandbox::new();
    let stub = env!("CARGO_BIN_EXE_opencode_stub");
    sandbox.stage_tool("opencode", &format!("#!/bin/sh\nexec '{stub}' \"$@\"\n"));
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("opencode_resume", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "OpenCode resume").await;
    let ws = text(&view, "/item/id");

    j.step("start an OpenCode session and tell it a nonce");
    let (session, prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            format!("Remember {NONCE}."),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Opencode),
                model: Some("opencode/stub-free".into()),
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
    let (_, first) = turn(&mut events, &prompt).await;
    let opened = harness_session(&mut j, &client, &ws, &id).await;
    j.alias(&opened, "$OPENCODE_SESSION");
    j.guard(
        "the first turn runs in a new OpenCode session",
        first.as_deref() == Some("model=stub-free earlier=[]") && opened.starts_with("ses_"),
        json!({ "reply": first, "harnessSession": opened }),
    );

    j.step("a resume after a server restart continues the same OpenCode session");
    drop(events);
    let stopped = server.shutdown(&client).await;
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events = resume(&client, &id).await;
    let asked = "What was the nonce?";
    send(&mut j, &client, &id, asked).await;
    let (_, after_restart) = turn(&mut events, asked).await;
    let resumed = harness_session(&mut j, &client, &ws, &id).await;
    j.guard(
        "the resumed turn sees the first turn's nonce in the same OpenCode session",
        stopped.status.success()
            && resumed == opened
            && recalls(&after_restart)
            && model_of(&after_restart) == Some("stub-free"),
        json!({ "reply": after_restart, "harnessSession": resumed }),
    );

    j.step("a restart on another model continues it on that model");
    drop(events);
    let (_, _, _, mut events) = client
        .restart_session_stream(
            id.clone(),
            Some("opencode/stub-other-free".into()),
            None,
            None,
        )
        .await
        .expect("restart_session");
    let again = "Repeat the nonce.";
    send(&mut j, &client, &id, again).await;
    let (_, switched) = turn(&mut events, again).await;
    let restarted = harness_session(&mut j, &client, &ws, &id).await;
    j.guard(
        "the turn runs on the new model and still sees the nonce",
        restarted == opened && recalls(&switched) && model_of(&switched) == Some("stub-other-free"),
        json!({ "reply": switched, "harnessSession": restarted }),
    );

    // The echo lands after the new driver subscribes, the abort's idle after the echo, and the
    // next turn's user message after both.
    j.step("a relaunch right after a turn ignores the echo and the abort that trail it");
    stub_delays(
        &sandbox,
        json!({ "echo": 1000, "abortIdle": 1400, "start": 2200 }),
    );
    let before = "One more before the relaunch.";
    send(&mut j, &client, &id, before).await;
    turn(&mut events, before).await;
    drop(events);
    let mut events = restart(&client, &id).await;
    let first = "First after the relaunch.";
    send(&mut j, &client, &id, first).await;
    let (_, relaunched) = turn(&mut events, first).await;
    let extra = turn_ends(&mut events, Duration::from_millis(1500)).await;
    j.guard(
        "the first turn after the relaunch ends once, with its own reply",
        recalls(&relaunched) && extra == 0,
        json!({ "reply": relaunched, "laterTurnEnds": extra }),
    );

    j.step("a send right after a mid-turn restart outlasts the aborted turn's late idle");
    stub_delays(&sandbox, json!({ "reply": 2000, "late": 800 }));
    let long = "A long turn.";
    send(&mut j, &client, &id, long).await;
    sent(&mut events, long).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(events);
    let mut events = restart(&client, &id).await;
    let next = "First after the mid-turn restart.";
    send(&mut j, &client, &id, next).await;
    let (_, after_abort) = turn(&mut events, next).await;
    let extra = turn_ends(&mut events, Duration::from_millis(1500)).await;
    j.guard(
        "the turn ends once, with its own reply",
        recalls(&after_abort) && extra == 0,
        json!({ "reply": after_abort, "laterTurnEnds": extra }),
    );

    j.step("a turn whose permission the user denies ends");
    stub_delays(&sandbox, json!({ "ask": true }));
    send(&mut j, &client, &id, "Run a command.").await;
    let request_id = permission_ask(&mut events).await;
    j.call(
        &client,
        Request::RespondPermission {
            session_id: id.clone(),
            request_id: request_id.clone().unwrap_or_default(),
            allow: false,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
    let ended = turn_status(&mut events).await;
    j.guard(
        "the denied turn ends as completed",
        request_id.is_some() && ended.as_deref() == Some("completed"),
        json!({ "asked": request_id.is_some(), "turnStatus": ended }),
    );

    j.step("a subagent's permission ask shows, and allowing it lets the turn finish");
    stub_delays(&sandbox, json!({ "ask": true, "child": true }));
    send(&mut j, &client, &id, "Have a subagent run a command.").await;
    let delegated = permission_ask(&mut events).await;
    if let Some(request_id) = &delegated {
        client
            .respond_permission(id.clone(), request_id.clone(), true, None)
            .await
            .expect("respond_permission");
    }
    let ended = turn_status(&mut events).await;
    stub_delays(&sandbox, json!({}));
    j.guard(
        "the subagent's ask reaches the user, and the allowed turn completes",
        delegated.is_some() && ended.as_deref() == Some("completed"),
        json!({ "asked": delegated.is_some(), "turnStatus": ended }),
    );

    j.step("a tail read carries the asks OpenCode still waits on, until they end");
    stub_delays(
        &sandbox,
        json!({ "question": true, "ask": true, "aside": true }),
    );
    send(&mut j, &client, &id, "Ask, then run a command.").await;
    let question = ask_behind_aside(&mut events).await;
    let question_carried =
        open_asks(&client, &id).await == question.iter().cloned().collect::<Vec<_>>();
    let mut answers = BTreeMap::new();
    answers.insert("Proceed?".to_string(), "Yes".to_string());
    client
        .answer_question(
            id.clone(),
            question.clone().unwrap_or_default(),
            QuestionAnswers(answers),
        )
        .await
        .expect("answer_question");
    let permission = ask_behind_aside(&mut events).await;
    let permission_carried =
        open_asks(&client, &id).await == permission.iter().cloned().collect::<Vec<_>>();
    client
        .respond_permission(
            id.clone(),
            permission.clone().unwrap_or_default(),
            true,
            None,
        )
        .await
        .expect("respond_permission");
    let answered = open_asks(&client, &id).await.is_empty();
    let ended = turn_status(&mut events).await;
    stub_delays(&sandbox, json!({ "ask": true, "aside": true }));
    send(&mut j, &client, &id, "Run a command you'll be stopped on.").await;
    let cancelled = ask_behind_aside(&mut events).await;
    let cancel_carried =
        open_asks(&client, &id).await == cancelled.iter().cloned().collect::<Vec<_>>();
    client
        .interrupt_session(id.clone())
        .await
        .expect("interrupt_session");
    let dropped = open_asks(&client, &id).await.is_empty();
    let stopped = turn_status(&mut events).await;
    stub_delays(&sandbox, json!({}));
    j.guard(
        "each ask is carried while it waits, and dropped once answered or cancelled",
        question.is_some()
            && question_carried
            && permission.is_some()
            && permission_carried
            && answered
            && ended.as_deref() == Some("completed")
            && cancelled.is_some()
            && cancel_carried
            && dropped
            && stopped.as_deref() == Some("interrupted"),
        json!({
            "questionCarried": question_carried,
            "permissionCarried": permission_carried,
            "answeredDropped": answered,
            "turnStatus": ended,
            "cancelCarried": cancel_carried,
            "cancelDropped": dropped,
            "cancelledStatus": stopped,
        }),
    );

    j.step("the log closes each ask once: answered, denied, or cancelled by a stop");
    client
        .announce_client(
            "desktop".into(),
            "e2e".into(),
            TARGETED_EVENTS.map(str::to_string).to_vec(),
        )
        .await
        .expect("announce_client");
    let named = [
        (request_id, "denied"),
        (delegated, "subagent"),
        (question, "question"),
        (permission, "permission"),
        (cancelled, "cancelled"),
    ];
    let closed: Vec<serde_json::Value> = client
        .get_session_log(id.clone())
        .await
        .expect("get_session_log")
        .into_iter()
        .filter(|e| matches!(e.kind.as_str(), "permissionResolved" | "questionAnswered"))
        .filter_map(|e| serde_json::to_value(e.event).ok())
        .map(|mut row| {
            let ask = named
                .iter()
                .find(|(id, _)| id.as_deref() == row["requestId"].as_str())
                .map_or("unknown", |(_, name)| *name);
            row["requestId"] = json!(ask);
            row
        })
        .collect();
    j.guard(
        "each ask has one closing row, with how it closed",
        closed
            == [
                json!({ "kind": "permissionResolved", "requestId": "denied", "allow": false }),
                json!({ "kind": "permissionResolved", "requestId": "subagent", "allow": true }),
                json!({ "kind": "questionAnswered", "requestId": "question" }),
                json!({ "kind": "permissionResolved", "requestId": "permission", "allow": true }),
                json!({ "kind": "permissionResolved", "requestId": "cancelled", "allow": false }),
            ],
        json!({ "closed": closed }),
    );

    j.step("an ask OpenCode raises as the stop lands closes with the stopped turn");
    stub_delays(&sandbox, json!({ "reply": 2000, "lateAsk": true }));
    let late = "A turn stopped as it asks.";
    send(&mut j, &client, &id, late).await;
    sent(&mut events, late).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    client
        .interrupt_session(id.clone())
        .await
        .expect("interrupt_session");
    let stopped = turn_status(&mut events).await;
    let left_open = open_asks(&client, &id).await;
    stub_delays(&sandbox, json!({}));
    j.guard(
        "the turn ends interrupted, with no ask left open",
        stopped.as_deref() == Some("interrupted") && left_open.is_empty(),
        json!({ "turnStatus": stopped, "openAsks": left_open.len() }),
    );

    j.step("an OpenCode that ignores the prompt's messageID still ends each turn");
    stub_delays(&sandbox, json!({ "ownIds": true }));
    let own = "Answer under OpenCode's own ids.";
    send(&mut j, &client, &id, own).await;
    let (seen, replied) = turn(&mut events, own).await;
    j.guard(
        "its reply ends the turn",
        replied.is_some() && status_of(&seen).as_deref() == Some("completed"),
        json!({ "replied": replied.is_some(), "turnStatus": status_of(&seen) }),
    );
    stub_delays(&sandbox, json!({ "ownIds": true, "fail": "model gone" }));
    let failing = "Fail before replying.";
    send(&mut j, &client, &id, failing).await;
    let (seen, said) = turn(&mut events, failing).await;
    let extra = turn_ends(&mut events, Duration::from_millis(1500)).await;
    j.guard(
        "a turn that fails before any reply ends once, as failed",
        status_of(&seen).as_deref() == Some("failed") && extra == 0,
        json!({ "said": said, "turnStatus": status_of(&seen), "laterTurnEnds": extra }),
    );
    stub_delays(&sandbox, json!({ "bare": true }));
    let bare = "Answer under ids the driver cannot place.";
    send(&mut j, &client, &id, bare).await;
    let (seen, replied) = turn(&mut events, bare).await;
    let notice = notices(&seen);
    j.guard(
        "the turn still ends, and the transcript says why",
        replied.is_some() && status_of(&seen).as_deref() == Some("completed") && notice.len() == 1,
        json!({ "replied": replied.is_some(), "turnStatus": status_of(&seen), "notice": notice }),
    );
    stub_delays(&sandbox, json!({}));

    j.step(
        "an OpenCode that names the session only inside each message ends the turn on its reply",
    );
    stub_delays(&sandbox, json!({ "nested": true }));
    let nested = "Answer with the session named only inside the message.";
    send(&mut j, &client, &id, nested).await;
    let (seen, replied) = turn(&mut events, nested).await;
    let notice = notices(&seen);
    j.guard(
        "its reply ends the turn, with no fallback notice",
        replied.is_some() && status_of(&seen).as_deref() == Some("completed") && notice.is_empty(),
        json!({ "replied": replied.is_some(), "turnStatus": status_of(&seen), "notice": notice }),
    );
    stub_delays(&sandbox, json!({}));

    j.step("a resume after OpenCode's data is wiped starts a new session and says so");
    drop(events);
    let stopped = server.shutdown(&client).await;
    std::fs::remove_dir_all(sandbox.app_data.join("opencode")).expect("wipe OpenCode's data");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events = resume(&client, &id).await;
    let lost = "Do you still know the nonce?";
    send(&mut j, &client, &id, lost).await;
    let (seen, after_wipe) = turn(&mut events, lost).await;
    let replaced = harness_session(&mut j, &client, &ws, &id).await;
    let notice = notices(&seen);
    j.guard(
        "the turn runs in a new OpenCode session, and the transcript says the context is gone",
        stopped.status.success()
            && replaced != opened
            && replaced.starts_with("ses_")
            && after_wipe.is_some()
            && !recalls(&after_wipe)
            && notice.len() == 1,
        json!({ "reply": after_wipe, "replacedSession": replaced != opened, "notice": notice }),
    );

    j.step(
        "a retried start that never got an OpenCode session creates one, and a restart reuses it",
    );
    drop(events);
    let (retried, prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "Start on a model OpenCode lacks.".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Opencode),
                model: Some("opencode/stub-missing-free".into()),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    let retried = retried.id;
    j.alias(&retried, "$RETRIED");
    let launched = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: retried.clone(),
            },
        )
        .await;
    let exited = turn_ends_or_exit(&mut events).await;
    let before = harness_session(&mut j, &client, &ws, &retried).await;
    j.call(
        &client,
        Request::SetSessionModel {
            session_id: retried.clone(),
            model: Some("opencode/stub-free".into()),
        },
    )
    .await
    .expect("set_session_model");
    drop(events);
    let mut events = resume(&client, &retried).await;
    let (_, replayed) = turn(&mut events, &prompt).await;
    let created = harness_session(&mut j, &client, &ws, &retried).await;
    j.guard(
        "the retry replays the stored prompt into a new OpenCode session",
        launched.is_err()
            && exited
            && before.is_empty()
            && created.starts_with("ses_")
            && replayed.as_deref() == Some("model=stub-free earlier=[]"),
        json!({
            "launchFailed": launched.is_err(),
            "exited": exited,
            "sessionBefore": before,
            "sessionAfter": created.starts_with("ses_"),
            "reply": replayed,
        }),
    );
    j.alias(&created, "$RETRIED_SESSION");
    drop(events);
    let mut events = restart(&client, &retried).await;
    let still = "Still there?";
    send(&mut j, &client, &retried, still).await;
    let (_, kept) = turn(&mut events, still).await;
    let reused = harness_session(&mut j, &client, &ws, &retried).await;
    j.guard(
        "a restart continues that OpenCode session",
        reused == created
            && kept
                .as_deref()
                .is_some_and(|r| r.ends_with(&format!("earlier=[{prompt}]"))),
        json!({ "harnessSession": reused, "reply": kept }),
    );

    j.step("when opencode serve dies mid-turn, each session on it ends, and a resume continues");
    stub_delays(&sandbox, json!({ "reply": 30000 }));
    let stranded = "A turn the server dies in.";
    send(&mut j, &client, &retried, stranded).await;
    sent(&mut events, stranded).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let deadline = kill_serve(&sandbox) + Duration::from_secs(5);
    let exited = exit_by(&mut events, deadline).await;
    let rows = crashed_by(&client, &ws, [&id, &retried], deadline).await;
    stub_delays(&sandbox, json!({}));
    j.guard(
        "within 5s both sessions on it end as crashed, and neither reads as working",
        exited == Some(ExitOutcome::Crashed)
            && rows == [json!(["crashed", false]), json!(["crashed", false])],
        json!({ "exited": exited, "sessions": rows }),
    );
    drop(events);
    let mut events = resume(&client, &retried).await;
    let back = "Back after the crash?";
    send(&mut j, &client, &retried, back).await;
    let (_, after_crash) = turn(&mut events, back).await;
    let continued = harness_session(&mut j, &client, &ws, &retried).await;
    j.guard(
        "the resume starts a new server and continues the same OpenCode session",
        continued == created
            && after_crash
                .as_deref()
                .is_some_and(|r| r.ends_with(&format!("{still} | {stranded}]"))),
        json!({ "harnessSession": continued, "reply": after_crash }),
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

fn recalls(reply: &Option<String>) -> bool {
    reply.as_deref().is_some_and(|r| r.contains(NONCE))
}

fn model_of(reply: &Option<String>) -> Option<&str> {
    reply.as_deref()?.strip_prefix("model=")?.split(' ').next()
}

fn stub_delays(sandbox: &Sandbox, delays: serde_json::Value) {
    let control = sandbox.app_data.join("opencode/opencode-stub/control.json");
    std::fs::write(control, delays.to_string()).expect("write the stub's delays");
}

/// An in-place relaunch on the model the session already runs.
async fn restart(client: &RpcClient, id: &str) -> EventStream<SessionEvent> {
    let (_, _, _, events) = client
        .restart_session_stream(
            id.to_string(),
            Some("opencode/stub-other-free".into()),
            None,
            None,
        )
        .await
        .expect("restart_session");
    events
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

/// The row's stored harness session id.
async fn harness_session(j: &mut Journey, client: &RpcClient, ws: &str, id: &str) -> String {
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
        .and_then(|s| s["claudeSessionId"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// Every event up to the end of the turn `sent` started, and its reply; a replayed log
/// carries earlier turns, so only the text after `sent`'s own user message counts.
async fn turn(
    events: &mut EventStream<SessionEvent>,
    sent: &str,
) -> (Vec<SessionEvent>, Option<String>) {
    let mut seen = Vec::new();
    let mut reply = None;
    let _ = tokio::time::timeout(IO, async {
        let mut started = false;
        while let Some(event) = events.recv().await {
            match &event {
                SessionEvent::UserMessage { text, .. } if text == sent => started = true,
                SessionEvent::AssistantText { text, .. } if started => reply = Some(text.clone()),
                SessionEvent::TurnDone { .. } if started => {
                    seen.push(event);
                    return;
                }
                _ => {}
            }
            seen.push(event);
        }
    })
    .await;
    (seen, reply)
}

/// Waits for the user message `text`, as the driver reports it on send.
async fn sent(events: &mut EventStream<SessionEvent>, text: &str) {
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(&event, SessionEvent::UserMessage { text: said, .. } if said == text) {
                return;
            }
        }
    })
    .await;
}

/// The turn ends that arrive within `window`, where a turn that ended once should see none.
async fn turn_ends(events: &mut EventStream<SessionEvent>, window: Duration) -> usize {
    let mut ends = 0;
    let _ = tokio::time::timeout(window, async {
        while let Some(event) = events.recv().await {
            ends += usize::from(matches!(event, SessionEvent::TurnDone { .. }));
        }
    })
    .await;
    ends
}

async fn permission_ask(events: &mut EventStream<SessionEvent>) -> Option<String> {
    tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if let SessionEvent::PermissionAsk { request_id, .. } = event {
                return Some(request_id);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// The request id of the next ask, once the text after it has made it older than the newest row.
async fn ask_behind_aside(events: &mut EventStream<SessionEvent>) -> Option<String> {
    tokio::time::timeout(IO, async {
        let mut asked = None;
        while let Some(event) = events.recv().await {
            match event {
                SessionEvent::PermissionAsk { request_id, .. }
                | SessionEvent::QuestionAsk { request_id, .. } => asked = Some(request_id),
                SessionEvent::AssistantText { .. } if asked.is_some() => return asked,
                _ => {}
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// The request ids a one-row tail read carries as still open.
async fn open_asks(client: &RpcClient, id: &str) -> Vec<String> {
    let page = client
        .list_session_turns(id.to_string(), LogCursor::Tail, 1)
        .await
        .expect("list_session_turns");
    page.open_asks
        .iter()
        .filter_map(|ask| match &ask.event {
            SessionEvent::PermissionAsk { request_id, .. }
            | SessionEvent::QuestionAsk { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
        .collect()
}

async fn turn_status(events: &mut EventStream<SessionEvent>) -> Option<String> {
    tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if let SessionEvent::TurnDone { turn_status, .. } = event {
                return turn_status;
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn notices(seen: &[SessionEvent]) -> Vec<&str> {
    seen.iter()
        .filter_map(|e| match e {
            SessionEvent::ProtocolError { detail, .. } => Some(detail.as_str()),
            _ => None,
        })
        .collect()
}

fn status_of(seen: &[SessionEvent]) -> Option<String> {
    seen.iter().rev().find_map(|e| match e {
        SessionEvent::TurnDone { turn_status, .. } => turn_status.clone(),
        _ => None,
    })
}

fn kill_serve(sandbox: &Sandbox) -> Instant {
    let record = std::fs::read_to_string(sandbox.app_data.join("opencode-server.pid"))
        .expect("read the opencode serve pid");
    let pid = record.lines().next().unwrap_or_default();
    let killed = std::process::Command::new("kill")
        .args(["-9", pid])
        .status()
        .is_ok_and(|s| s.success());
    assert!(killed, "kill opencode serve {pid}");
    Instant::now()
}

async fn exit_by(events: &mut EventStream<SessionEvent>, deadline: Instant) -> Option<ExitOutcome> {
    tokio::time::timeout_at(deadline, async {
        while let Some(event) = events.recv().await {
            if let SessionEvent::Exited { outcome, .. } = event {
                return Some(outcome);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

async fn crashed_by(
    client: &RpcClient,
    ws: &str,
    ids: [&str; 2],
    deadline: Instant,
) -> Vec<serde_json::Value> {
    loop {
        let sessions = client
            .list_sessions(ws.to_string())
            .await
            .expect("list_sessions");
        let rows: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| match sessions.iter().find(|s| s.id == *id) {
                Some(s) => json!([s.state, s.working]),
                None => json!(null),
            })
            .collect();
        if rows.iter().all(|row| row[0] == "crashed") || Instant::now() >= deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Whether the session's harness exited, as a start that failed before it launched does.
async fn turn_ends_or_exit(events: &mut EventStream<SessionEvent>) -> bool {
    tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(event, SessionEvent::Exited { .. }) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}
