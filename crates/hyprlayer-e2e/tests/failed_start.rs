//! A start that never reaches a harness ends as a visible failure that survives a
//! reload: the log keeps the prompt and the reason, whichever step broke. Retrying
//! it starts the harness fresh and sends that prompt, rather than resuming nothing.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::models::{ExitOutcome, SessionOrigin};
use hyprlayer_core::pipeline;
use hyprlayer_core::store::{Db, NewSession};
use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_proto::{Outcome, PROTOCOL_VERSION};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::rpc::WatchedSession;
use hyprlayer_server::streams::{StreamRequest, StreamResponse, StreamStartOpts};
use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};
use tokio::net::UnixStream;

const IO: Duration = Duration::from_secs(20);

/// OpenCode refuses this provider before any process starts, so no binary is needed.
const UNLAUNCHABLE_MODEL: &str = "github-copilot/gpt-5";
const PROMPT: &str = "E2E_UNSENT_PROMPT";
const CLIENT_MESSAGE_ID: &str = "e2e-first-turn";
const PICKED_MODEL: &str = "claude-e2e-picked";
const PICKED_EFFORT: &str = "high";

/// Logs `new|resume <id> <model> <effort>` per spawn to `<sandbox>/spawns.log` and every stdin line
/// to `<sandbox>/harness.log`. `<sandbox>/fake-mode` changes it: `die` closes stdin and
/// exits before reading a turn, `silent` reads turns but never prints its init or a
/// reply, and `quiet-crash` reads one turn without an init and exits.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
how="new"
model="-"
effort="-"
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    --resume) sid="$2"; how="resume"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    --effort) effort="$2"; shift 2 ;;
    *) shift ;;
  esac
done
root="$(dirname "$0")/.."
[ -n "$sid" ] && printf '%s %s %s %s\n' "$how" "$sid" "$model" "$effort" >> "$root/spawns.log"
mode="$(cat "$root/fake-mode" 2>/dev/null)"
if [ "$mode" = "quiet-crash" ]; then
  IFS= read -r line && printf '%s\n' "$line" >> "$root/harness.log"
  exit 3
fi
if [ "$mode" = "die" ]; then
  exec 0<&-
  echo "E2E_HARNESS_DIED" >&2
  sleep 0.3
  exit 3
fi
if [ "$mode" = "silent" ]; then
  while IFS= read -r line; do printf '%s\n' "$line" >> "$root/harness.log"; done
  exit 0
fi
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$root/harness.log"
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_start() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("failed_start", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Failed start").await;
    let ws = text(&view, "/item/id");

    j.step("start a session whose harness cannot launch");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            PROMPT.into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Opencode),
                model: Some(UNLAUNCHABLE_MODEL.into()),
                client_message_id: Some(CLIENT_MESSAGE_ID.into()),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$SESSION");
    let launched = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: session.id.clone(),
            },
        )
        .await;
    let code = launched.as_ref().err().map(|e| e.code.clone());
    j.guard(
        "the caller still hears why the launch failed",
        code.as_deref() == Some("opencode_provider_unsupported"),
        json!({ "code": code }),
    );
    let streamed = recv_until(&mut events, |e| matches!(e, SessionEvent::Exited { .. })).await;
    j.guard(
        "the stream echoes the prompt it could not send",
        streamed.iter().any(|e| {
            matches!(e, SessionEvent::UserMessage { client_message_id: Some(id), .. } if id == CLIENT_MESSAGE_ID)
        }),
        json!({ "kinds": event_kinds(&streamed) }),
    );
    j.guard(
        "the stream ends with a failed start",
        streamed.iter().any(|e| {
            matches!(e, SessionEvent::Exited { outcome: ExitOutcome::SpawnFailed, stderr_tail, .. } if !stderr_tail.is_empty())
        }),
        json!({ "kinds": event_kinds(&streamed) }),
    );

    j.step("a reader that never saw the stream sees the failure");
    let sessions = j
        .call(
            &client,
            Request::ListSessions {
                workstream_id: ws.clone(),
            },
        )
        .await
        .expect("list_sessions");
    let row = sessions
        .as_array()
        .and_then(|rows| rows.iter().find(|r| text(r, "/id") == session.id))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the row ends spawn_failed, not starting",
        text(&row, "/state") == "spawn_failed",
        json!({ "state": row.pointer("/state") }),
    );
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let rows = log.as_array().cloned().unwrap_or_default();
    let kinds: Vec<String> = rows.iter().map(|r| text(r, "/event/kind")).collect();
    j.guard(
        "the log keeps the prompt",
        rows.iter().any(|r| {
            text(r, "/event/kind") == "userMessage" && text(r, "/event/text").contains(PROMPT)
        }),
        json!({ "kinds": kinds }),
    );
    let reason = rows
        .iter()
        .find(|r| text(r, "/event/kind") == "exited")
        .map(|r| (text(r, "/event/outcome"), text(r, "/event/stderrTail")));
    j.guard(
        "the log ends with the failed start and its reason",
        reason.as_ref().is_some_and(|(outcome, tail)| {
            outcome == "spawn_failed" && tail.contains("GitHub Copilot")
        }),
        json!({ "exit": reason }),
    );

    j.step("retry a claude start that failed before claude was installed");
    let (missing, ship_prompt, mut missing_events) = start_session(
        &client,
        &ws,
        Stage::Ship,
        "E2E_RETRY_PROMPT",
        Some(PICKED_MODEL),
        Some(PICKED_EFFORT),
    )
    .await;
    j.alias(&missing, "$MISSING");
    let _ = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: missing.clone(),
            },
        )
        .await;
    recv_until(&mut missing_events, |e| {
        matches!(e, SessionEvent::Exited { .. })
    })
    .await;
    let failed = failure_record(&mut j, &client, &ws, &missing).await;
    j.guard(
        "the missing tool ends the start as a failure, with an end time",
        failed["state"] == "spawn_failed" && failed["exits"] == 1 && failed["ended"] == true,
        failed.clone(),
    );
    let first_end = ended_at(&mut j, &client, &ws, &missing).await;
    let viewer = server.connect().await;
    let mut viewer_events = viewer.take_events().expect("the viewer's runtime events");
    j.call(
        &viewer,
        Request::AnnounceClient {
            client_id: "e2e-viewer".into(),
            device: "viewer".into(),
            capabilities: TARGETED_EVENTS.map(str::to_string).to_vec(),
        },
    )
    .await
    .expect("announce_client (viewer)");
    let seen = j
        .call(&viewer, list_turns(&missing, LogCursor::Empty))
        .await
        .expect("list_session_turns (viewer)");
    let seen_rev = rev_of(&seen);
    let viewer_cursor: LogCursor = seen
        .pointer("/nextCursor")
        .cloned()
        .and_then(|c| serde_json::from_value(c).ok())
        .expect("the viewer's cursor");
    j.call(&viewer, watch(&missing, seen_rev))
        .await
        .expect("watch_sessions (viewer)");
    let refused = client
        .resume_session_stream(missing.clone(), None, None, None, None)
        .await
        .is_err();
    let deadline = tokio::time::Instant::now() + IO;
    let mut again = failure_record(&mut j, &client, &ws, &missing).await;
    while again["exits"] != 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        again = failure_record(&mut j, &client, &ws, &missing).await;
    }
    let later = ended_at(&mut j, &client, &ws, &missing).await > first_end;
    j.guard(
        "a retry that fails again is recorded like the start: another failed exit and a later end time",
        refused
            && again["state"] == "spawn_failed"
            && again["exits"] == 2
            && again["failed"] == true
            && later,
        json!({ "refused": refused, "after": again, "later_end": later }),
    );

    let revised_past = |e: &RuntimeEvent| {
        matches!(e, RuntimeEvent::SessionRevised { session_id, rev, .. }
            if *session_id == missing && *rev > seen_rev)
    };
    let told = events_until(&mut viewer_events, IO, revised_past)
        .await
        .iter()
        .any(revised_past);
    let polled = j
        .call(&viewer, watch(&missing, seen_rev))
        .await
        .expect("watch_sessions (viewer, after)");
    let polled_rev = polled
        .as_array()
        .and_then(|replies| replies.first())
        .map_or(0, rev_of);
    let pulled = j
        .call(&viewer, list_turns(&missing, viewer_cursor))
        .await
        .expect("list_session_turns (viewer, after)");
    let new_rows: Vec<(String, String)> = pulled
        .pointer("/entries")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|r| (text(r, "/event/kind"), text(r, "/event/outcome")))
                .collect()
        })
        .unwrap_or_default();
    j.guard(
        "a viewer is told of the refusal's exit and pulls exactly that row",
        told && polled_rev > seen_rev
            && rev_of(&pulled) == polled_rev
            && new_rows == [("exited".to_string(), "spawn_failed".to_string())],
        json!({
            "told": told,
            "watch_moved": polled_rev > seen_rev,
            "new_rows": new_rows,
        }),
    );
    sandbox.install_absent("claude", FAKE_CLAUDE);
    j.call(&client, Request::RedetectBinaries {})
        .await
        .expect("redetect_binaries");
    let (_, _, _, mut resumed) = client
        .resume_session_stream(missing.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    let mut replied =
        recv_until(&mut resumed, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    replied.extend(recv_until(&mut resumed, |e| matches!(e, SessionEvent::TurnDone { .. })).await);
    let spawns = std::fs::read_to_string(sandbox.root.join("spawns.log")).unwrap_or_default();
    let spawned: Vec<&str> = spawns.lines().filter(|l| l.contains(&missing)).collect();
    j.guard(
        "the retry starts claude fresh instead of resuming a conversation it never had",
        !spawned.is_empty() && spawned.iter().all(|l| l.starts_with("new ")),
        json!({ "spawns": spawned.iter().map(|l| l.split(' ').next()).collect::<Vec<_>>() }),
    );
    let picked = format!(" {PICKED_MODEL} {PICKED_EFFORT}");
    j.guard(
        "the retry launches the model and effort the user picked",
        !spawned.is_empty() && spawned.iter().all(|l| l.ends_with(&picked)),
        json!({ "launched": spawned.iter().map(|l| l.split(' ').skip(2).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>() }),
    );
    let harness_log = sandbox.root.join("harness.log");
    let turns = claude_turns(&harness_log, &missing);
    let full = turns.iter().filter(|t| **t == ship_prompt).count();
    j.guard(
        "the retry sends the full stage prompt once",
        full == 1,
        json!({ "full_prompt_sent": full }),
    );
    j.guard(
        "the stage's follow-up runs after the retried turn",
        turns.get(1).map(String::as_str) == Some(pipeline::SHIP_DESCRIBE_PROMPT),
        json!({ "turns": turns.len() }),
    );
    let reply = replied.iter().any(
        |e| matches!(e, SessionEvent::AssistantText { text, .. } if text.starts_with("E2E_REPLY_")),
    );
    j.guard(
        "claude's reply reaches the client",
        reply,
        json!({ "replied": reply }),
    );
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: missing.clone(),
            text: "E2E_SECOND_PROMPT".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    recv_until(&mut resumed, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    let logged = user_rows(&mut j, &client, &missing).await;
    let read = claude_turns(&harness_log, &missing).len();
    j.guard(
        "the log holds one user row per turn claude read, so rewind and turn counts line up",
        logged == read,
        json!({ "logged": logged, "read": read }),
    );
    let live = failure_record(&mut j, &client, &ws, &missing).await;
    j.guard(
        "the retried row is running again with no end time or failure summary",
        live["state"] == "running" && live["ended"] == false && live["summary"] == false,
        json!({ "state": live["state"], "ended": live["ended"], "summary": live["summary"] }),
    );
    drop(resumed);

    j.step("retry the failed OpenCode start on claude");
    let moved = j
        .call(
            &client,
            Request::SetSessionHarness {
                session_id: session.id.clone(),
                harness: Harness::Claude,
                model: Some(PICKED_MODEL.into()),
                effort: None,
            },
        )
        .await;
    j.guard(
        "a start that never launched moves to another harness",
        moved.is_ok(),
        json!({ "code": moved.as_ref().err().map(|e| e.code.clone()) }),
    );
    let (_, _, _, mut moved_events) = client
        .resume_session_stream(session.id.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    let replied = recv_until(&mut moved_events, |e| {
        matches!(e, SessionEvent::TurnDone { .. })
    })
    .await;
    let spawns = std::fs::read_to_string(sandbox.root.join("spawns.log")).unwrap_or_default();
    let spawned: Vec<&str> = spawns.lines().filter(|l| l.contains(&session.id)).collect();
    let picked_only = format!(" {PICKED_MODEL} -");
    j.guard(
        "claude starts fresh on the picked model, with no effort left from the failed start",
        spawned.len() == 1 && spawned[0].starts_with("new ") && spawned[0].ends_with(&picked_only),
        json!({ "spawns": spawned.len(), "picked": spawned.iter().all(|l| l.ends_with(&picked_only)) }),
    );
    let turns = claude_turns(&harness_log, &session.id);
    j.guard(
        "claude reads the prompt the OpenCode start never sent, once",
        turns == [PROMPT],
        json!({ "turns": turns.len() }),
    );
    let reply = replied.iter().any(
        |e| matches!(e, SessionEvent::AssistantText { text, .. } if text.starts_with("E2E_REPLY_")),
    );
    j.guard(
        "claude's reply reaches the client",
        reply,
        json!({ "replied": reply }),
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
    let row = sessions
        .as_array()
        .and_then(|rows| rows.iter().find(|r| text(r, "/id") == session.id))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the row runs on claude now",
        text(&row, "/harness") == "claude" && text(&row, "/state") == "running",
        json!({ "harness": row.pointer("/harness"), "state": row.pointer("/state") }),
    );
    let again = j
        .call(
            &client,
            Request::SetSessionHarness {
                session_id: session.id.clone(),
                harness: Harness::Opencode,
                model: None,
                effort: None,
            },
        )
        .await;
    let code = again.as_ref().err().map(|e| e.code.clone());
    j.guard(
        "a session that ran keeps its harness",
        code.as_deref() == Some("session_launched"),
        json!({ "code": code }),
    );
    drop(moved_events);

    j.step("a stage prompt composed for OpenCode is sent as claude's");
    let (commit, _prompt, _stream_id, mut commit_events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::Commit,
            "E2E_COMMIT_INPUT".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Opencode),
                model: Some(UNLAUNCHABLE_MODEL.into()),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session (commit)");
    j.alias(&commit.id, "$COMMIT");
    let _ = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: commit.id.clone(),
            },
        )
        .await;
    recv_until(&mut commit_events, |e| {
        matches!(e, SessionEvent::Exited { .. })
    })
    .await;
    j.call(
        &client,
        Request::SetSessionHarness {
            session_id: commit.id.clone(),
            harness: Harness::Claude,
            model: None,
            effort: None,
        },
    )
    .await
    .expect("set_session_harness (commit)");
    let (_, _, _, mut commit_resumed) = client
        .resume_session_stream(commit.id.clone(), None, None, None, None)
        .await
        .expect("resume_session (commit)");
    recv_until(&mut commit_resumed, |e| {
        matches!(e, SessionEvent::TurnDone { .. })
    })
    .await;
    let turns = claude_turns(&harness_log, &commit.id);
    let first = turns.first().map(String::as_str).unwrap_or_default();
    j.guard(
        "claude reads claude's commit prompt, not the one OpenCode was given",
        first.starts_with(pipeline::COMMIT_PROMPT_DEFAULT)
            && !first.starts_with(pipeline::COMMIT_PROMPT_NO_ASK_TOOL)
            && first.contains("E2E_COMMIT_INPUT"),
        json!({ "turns": turns.len(), "claude_prompt": first.starts_with(pipeline::COMMIT_PROMPT_DEFAULT) }),
    );
    drop(commit_resumed);

    j.step("claude dies at its first turn");
    std::fs::write(sandbox.root.join("fake-mode"), "die").expect("set the fake's mode");
    let (died, mut died_events) = start_claude(&client, &ws, "E2E_DIED_PROMPT").await;
    j.alias(&died, "$DIED");
    let _ = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: died.clone(),
            },
        )
        .await;
    let streamed = recv_until(&mut died_events, |e| {
        matches!(e, SessionEvent::Exited { .. })
    })
    .await;
    std::fs::remove_file(sandbox.root.join("fake-mode")).expect("clear the fake's mode");
    j.guard(
        "the stream ends with a failure and its reason",
        streamed.iter().any(|e| {
            matches!(e, SessionEvent::Exited { outcome: ExitOutcome::SpawnFailed | ExitOutcome::Crashed, stderr_tail, .. } if !stderr_tail.is_empty())
        }),
        json!({ "ended": streamed.iter().any(|e| matches!(e, SessionEvent::Exited { .. })) }),
    );
    let failed = failure_record(&mut j, &client, &ws, &died).await;
    j.guard(
        "the log keeps the prompt and one failed exit with its reason",
        failed["failed"] == true && failed["prompt"] == true && failed["exits"] == 1,
        failed.clone(),
    );

    j.step("a replayed turn echoes under its stored row's id");
    let (_, _, _, mut died_resumed) = client
        .resume_session_stream(died.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    let echoed = recv_until(&mut died_resumed, |e| {
        matches!(e, SessionEvent::TurnDone { .. })
    })
    .await;
    let echo_id = echoed.iter().find_map(|e| match e {
        SessionEvent::UserMessage {
            client_message_id, ..
        } => client_message_id.clone(),
        _ => None,
    });
    let row_id = session_log(&mut j, &client, &died)
        .await
        .iter()
        .find(|r| text(r, "/event/kind") == "userMessage")
        .map(|r| text(r, "/event/clientMessageId"))
        .filter(|id| !id.is_empty());
    j.guard(
        "the echo carries the id of the row the desktop already shows",
        echo_id.is_some() && echo_id == row_id,
        json!({ "same_id": echo_id.is_some() && echo_id == row_id }),
    );
    let rows = user_rows(&mut j, &client, &died).await;
    j.guard(
        "the replay adds no user row",
        rows == 1,
        json!({ "user_rows": rows }),
    );
    drop(died_resumed);

    j.step("a retry after a rewind replays the turn the rewind kept");
    std::fs::write(sandbox.root.join("fake-mode"), "die").expect("set the fake's mode");
    let (rewound, mut rewound_events) = start_claude(&client, &ws, "E2E_REWOUND_PROMPT").await;
    j.alias(&rewound, "$REWOUND");
    let _ = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: rewound.clone(),
            },
        )
        .await;
    recv_until(&mut rewound_events, |e| {
        matches!(e, SessionEvent::Exited { .. })
    })
    .await;
    std::fs::write(sandbox.root.join("fake-mode"), "quiet-crash").expect("set the fake's mode");
    let (_, _, _, rewind_events) = client
        .rewind_session_stream(rewound.clone(), 1, None)
        .await
        .expect("rewind_session");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: rewound.clone(),
            text: "E2E_KEPT_PROMPT".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let deadline = tokio::time::Instant::now() + IO;
    while failure_record(&mut j, &client, &ws, &rewound).await["state"] != "crashed"
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(rewind_events);
    std::fs::remove_file(sandbox.root.join("fake-mode")).expect("clear the fake's mode");
    let (_, _, _, mut kept) = client
        .resume_session_stream(rewound.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    recv_until(&mut kept, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    let replayed = claude_turns(&sandbox.root.join("harness.log"), &rewound)
        .last()
        .cloned()
        .unwrap_or_default();
    j.guard(
        "the replay is the kept turn, not the start's rewound prompt",
        replayed == "E2E_KEPT_PROMPT",
        json!({ "kept": replayed == "E2E_KEPT_PROMPT", "rewound_prompt": replayed.contains("E2E_REWOUND_PROMPT") }),
    );
    drop(kept);

    j.step("a mode switch before claude's init does not resend the prompt");
    std::fs::write(sandbox.root.join("fake-mode"), "silent").expect("set the fake's mode");
    let (quiet, quiet_events) = start_claude(&client, &ws, "E2E_QUIET_PROMPT").await;
    j.alias(&quiet, "$QUIET");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: quiet.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let sent = || turns_with(&harness_log, "E2E_QUIET_PROMPT");
    let deadline = tokio::time::Instant::now() + IO;
    while sent() == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    drop(quiet_events);
    let (_, _, _, restarted) = client
        .restart_session_stream(quiet.clone(), None, None, None)
        .await
        .expect("restart_session");
    // A resend would follow the restart at once.
    tokio::time::sleep(Duration::from_secs(2)).await;
    std::fs::remove_file(sandbox.root.join("fake-mode")).expect("clear the fake's mode");
    j.guard(
        "the prompt reaches claude once across the restart",
        sent() == 1,
        json!({ "sent": sent() }),
    );
    drop(restarted);
    j.call(
        &client,
        Request::KillSession {
            session_id: quiet.clone(),
        },
    )
    .await
    .expect("kill_session");

    j.step("a second launch call leaves a healthy session running");
    let (healthy, mut healthy_events) = start_claude(&client, &ws, "E2E_HEALTHY_PROMPT").await;
    j.alias(&healthy, "$HEALTHY");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: healthy.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    recv_until(&mut healthy_events, |e| {
        matches!(e, SessionEvent::TurnDone { .. })
    })
    .await;
    let again = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: healthy.clone(),
            },
        )
        .await;
    j.guard(
        "the second call is refused",
        again.is_err(),
        json!({ "refused": again.is_err() }),
    );
    let record = failure_record(&mut j, &client, &ws, &healthy).await;
    let active = j
        .call(&client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    j.guard(
        "the session stays live with no exit recorded",
        record["exits"] == 0 && ids(&active, "/session/id").contains(&healthy),
        record.clone(),
    );
    drop(healthy_events);

    j.step("the app disconnects between the start and the launch");
    let (abandoned, abandoned_prompt) =
        start_and_disconnect(server.socket(), &ws, "E2E_ABANDONED_PROMPT").await;
    j.alias(&abandoned, "$ABANDONED");
    let deadline = tokio::time::Instant::now() + IO;
    let mut settled = failure_record(&mut j, &client, &ws, &abandoned).await;
    while settled["state"] != "spawn_failed" && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        settled = failure_record(&mut j, &client, &ws, &abandoned).await;
    }
    j.guard(
        "the server records the start its client left behind",
        settled["state"] == "spawn_failed"
            && settled["failed"] == true
            && settled["prompt"] == true
            && settled["exits"] == 1,
        settled.clone(),
    );
    let stored = first_user_text(&mut j, &client, &abandoned).await;
    j.guard(
        "it keeps the full stage prompt",
        stored == abandoned_prompt,
        json!({ "full_prompt": stored == abandoned_prompt }),
    );

    j.step("the app drops while its start is still being prepared");
    let before = session_ids(&mut j, &client, &ws).await;
    send_start_and_drop(server.socket(), &ws, "E2E_PREPARING_PROMPT").await;
    let deadline = tokio::time::Instant::now() + IO;
    let mut dropped = None;
    while tokio::time::Instant::now() < deadline {
        let fresh = session_ids(&mut j, &client, &ws)
            .await
            .into_iter()
            .find(|id| !before.contains(id));
        if let Some(id) = fresh {
            if failure_record(&mut j, &client, &ws, &id).await["state"] == "spawn_failed" {
                dropped = Some(id);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let state = match &dropped {
        Some(id) => failure_record(&mut j, &client, &ws, id).await,
        None => {
            let fresh = session_ids(&mut j, &client, &ws)
                .await
                .into_iter()
                .find(|id| !before.contains(id))
                .unwrap_or_default();
            failure_record(&mut j, &client, &ws, &fresh).await
        }
    };
    j.guard(
        "the start is recorded as failed, not left starting",
        state["state"] == "spawn_failed" && state["prompt"] == true && state["exits"] == 1,
        state.clone(),
    );

    j.step("a launch call the app sent just before it dropped still runs");
    let launched = start_launch_and_drop(server.socket(), &ws, "E2E_LAUNCHED_PROMPT").await;
    j.alias(&launched, "$LAUNCHED");
    let deadline = tokio::time::Instant::now() + IO;
    let mut answered = false;
    while !answered && tokio::time::Instant::now() < deadline {
        answered = session_log(&mut j, &client, &launched)
            .await
            .iter()
            .any(|r| text(r, "/event/kind") == "assistantText");
        if !answered {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let record = failure_record(&mut j, &client, &ws, &launched).await;
    j.guard(
        "the harness answers and the start is not recorded as abandoned",
        answered && record["state"] != "spawn_failed",
        json!({ "answered": answered, "state": record["state"] }),
    );

    j.step("the server stops between the start and the launch");
    let (orphan, orphan_prompt, orphan_events) = start_session(
        &client,
        &ws,
        Stage::Ship,
        "E2E_ORPHAN_PROMPT",
        Some(PICKED_MODEL),
        Some(PICKED_EFFORT),
    )
    .await;
    j.alias(&orphan, "$ORPHAN");
    drop(orphan_events);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the server stops",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );
    let goal_start = seed_unlaunched_goal_start(&sandbox, &ws);
    j.alias(&goal_start, "$GOAL_START");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let goal_rows = session_ids(&mut j, &client, &ws).await;
    j.guard(
        "a goal start it never ran is discarded, not left for a capless retry",
        !goal_rows.contains(&goal_start),
        json!({ "kept": goal_rows.contains(&goal_start) }),
    );
    let failed = failure_record(&mut j, &client, &ws, &orphan).await;
    j.guard(
        "the next server records the start that never ran, with its prompt",
        failed["state"] == "spawn_failed"
            && failed["failed"] == true
            && failed["prompt"] == true
            && failed["exits"] == 1
            && failed["ended"] == true,
        failed.clone(),
    );
    let late = j
        .call(
            &client,
            Request::LaunchAndConverse {
                session_id: orphan.clone(),
            },
        )
        .await;
    let after = failure_record(&mut j, &client, &ws, &orphan).await;
    j.guard(
        "a late launch call on the new server changes nothing",
        late.is_err() && after == failed,
        json!({ "refused": late.is_err(), "after": after }),
    );
    let (_, _, _, mut resumed) = client
        .resume_session_stream(orphan.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    recv_until(&mut resumed, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    let turns = claude_turns(&sandbox.root.join("harness.log"), &orphan);
    let spawns = std::fs::read_to_string(sandbox.root.join("spawns.log")).unwrap_or_default();
    let picked = format!(" {PICKED_MODEL} {PICKED_EFFORT}");
    let spawned: Vec<&str> = spawns.lines().filter(|l| l.contains(&orphan)).collect();
    let launched = !spawned.is_empty() && spawned.iter().all(|l| l.ends_with(&picked));
    j.guard(
        "its retry sends the full stage prompt on the picked model",
        turns.first() == Some(&orphan_prompt) && launched,
        json!({ "full_prompt": turns.first() == Some(&orphan_prompt), "picked_model": launched }),
    );
    let live = failure_record(&mut j, &client, &ws, &orphan).await;
    j.guard(
        "the retried row is running again with no end time or failure summary",
        live["state"] == "running" && live["ended"] == false && live["summary"] == false,
        json!({ "state": live["state"], "ended": live["ended"], "summary": live["summary"] }),
    );
    drop(resumed);

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn start_claude(
    client: &RpcClient,
    ws: &str,
    input: &str,
) -> (String, EventStream<SessionEvent>) {
    let (id, _prompt, events) = start_session(client, ws, Stage::General, input, None, None).await;
    (id, events)
}

/// A claude start's id, the full prompt it composed, and its stream.
async fn start_session(
    client: &RpcClient,
    ws: &str,
    stage: Stage,
    input: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> (String, String, EventStream<SessionEvent>) {
    let (session, prompt, _stream_id, events) = client
        .start_stage_session_stream(
            ws.to_string(),
            stage,
            input.into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                model: model.map(str::to_string),
                effort: effort.map(str::to_string),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    (session.id, prompt, events)
}

/// The desktop's start on a raw socket that closes before the launch call; a dropped `RpcClient` stays connected.
async fn start_and_disconnect(socket: &std::path::Path, ws: &str, input: &str) -> (String, String) {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);
    let hello = reader.next_control().await.expect("read hello");
    assert!(matches!(hello, Some(Control::Hello { .. })), "{hello:?}");
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: None,
            version: None,
            features: Vec::new(),
            stream_window: None,
        })
        .await
        .expect("hello");
    writer
        .write_control(&Control::StreamRequest {
            id: 1,
            body: StreamRequest::StartStageSession {
                workstream_id: ws.to_string(),
                stage: Stage::Ship,
                input: input.into(),
                attached_docs: vec![],
                opts: StreamStartOpts {
                    harness: Some(Harness::Claude),
                    ..Default::default()
                },
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
            },
        })
        .await
        .expect("start");
    loop {
        match reader.next_control().await.expect("read the start reply") {
            Some(Control::StreamResponse {
                id: 1,
                outcome:
                    Outcome::Ok(StreamResponse::StartStageSession {
                        session, prompt, ..
                    }),
            }) => return (session.id, prompt),
            Some(Control::StreamResponse { id: 1, .. }) => panic!("the start failed"),
            Some(_) => continue,
            None => panic!("the server closed before answering the start"),
        }
    }
}

/// The hello a raw client sends before anything else.
async fn raw_client(
    socket: &std::path::Path,
) -> (
    FrameReader<tokio::io::ReadHalf<UnixStream>>,
    FrameWriter<tokio::io::WriteHalf<UnixStream>>,
) {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);
    let hello = reader.next_control().await.expect("read hello");
    assert!(matches!(hello, Some(Control::Hello { .. })), "{hello:?}");
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: None,
            version: None,
            features: Vec::new(),
            stream_window: None,
        })
        .await
        .expect("hello");
    (reader, writer)
}

fn start_request(ws: &str, input: &str) -> Control {
    Control::StreamRequest {
        id: 1,
        body: StreamRequest::StartStageSession {
            workstream_id: ws.to_string(),
            stage: Stage::General,
            input: input.into(),
            attached_docs: vec![],
            opts: StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
            echo_user_messages: true,
            with_meta: false,
            all_partials: false,
        },
    }
}

/// Closes the connection right after asking for the start, while the server still prepares it.
async fn send_start_and_drop(socket: &std::path::Path, ws: &str, input: &str) {
    let (_reader, mut writer) = raw_client(socket).await;
    writer
        .write_control(&start_request(ws, input))
        .await
        .expect("start");
}

/// Closes the connection right after sending the launch call for its start.
async fn start_launch_and_drop(socket: &std::path::Path, ws: &str, input: &str) -> String {
    let (mut reader, mut writer) = raw_client(socket).await;
    writer
        .write_control(&start_request(ws, input))
        .await
        .expect("start");
    let session_id = loop {
        match reader.next_control().await.expect("read the start reply") {
            Some(Control::StreamResponse {
                id: 1,
                outcome: Outcome::Ok(StreamResponse::StartStageSession { session, .. }),
            }) => break session.id,
            Some(Control::StreamResponse { id: 1, .. }) => panic!("the start failed"),
            Some(_) => continue,
            None => panic!("the server closed before answering the start"),
        }
    };
    writer
        .write_control(&Control::Request {
            id: 2,
            body: Request::LaunchAndConverse {
                session_id: session_id.clone(),
            },
        })
        .await
        .expect("launch");
    session_id
}

/// A goal's start row as the server leaves it when it stops before the launch; written while it is down.
fn seed_unlaunched_goal_start(sandbox: &Sandbox, ws: &str) -> String {
    let db = Db::open(&sandbox.app_data.join("hyprlayer.db")).expect("open the profile db");
    let id = "e2e-unlaunched-goal-start".to_string();
    db.insert_session(NewSession {
        id: id.clone(),
        workstream_id: ws.to_string(),
        stage: Stage::General,
        harness: Harness::Claude,
        cwd: sandbox.root.display().to_string(),
        first_input: Some("E2E_GOAL_PROMPT".into()),
    })
    .expect("insert the goal's start");
    db.set_session_lineage(&id, SessionOrigin::Goal, None)
        .expect("mark it a goal's start");
    id
}

async fn session_ids(j: &mut Journey, client: &RpcClient, ws: &str) -> Vec<String> {
    let sessions = j
        .call(
            client,
            Request::ListSessions {
                workstream_id: ws.to_string(),
            },
        )
        .await
        .expect("list_sessions");
    ids(&sessions, "/id")
}

async fn session_log(j: &mut Journey, client: &RpcClient, id: &str) -> Vec<Value> {
    let log = j
        .call(
            client,
            Request::GetSessionLog {
                session_id: id.to_string(),
            },
        )
        .await
        .expect("get_session_log");
    log.as_array().cloned().unwrap_or_default()
}

async fn user_rows(j: &mut Journey, client: &RpcClient, id: &str) -> usize {
    session_log(j, client, id)
        .await
        .iter()
        .filter(|r| text(r, "/event/kind") == "userMessage")
        .count()
}

async fn first_user_text(j: &mut Journey, client: &RpcClient, id: &str) -> String {
    session_log(j, client, id)
        .await
        .iter()
        .find(|r| text(r, "/event/kind") == "userMessage")
        .map(|r| text(r, "/event/text"))
        .unwrap_or_default()
}

/// What this session's claude read, in order: stream-json user turns, not the title request.
fn claude_turns(harness_log: &std::path::Path, session_id: &str) -> Vec<String> {
    std::fs::read_to_string(harness_log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| text(v, "/type") == "user" && text(v, "/session_id") == session_id)
        .map(|v| text(&v, "/message/content"))
        .collect()
}

async fn ended_at(j: &mut Journey, client: &RpcClient, ws: &str, id: &str) -> f64 {
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
        .and_then(|r| r.pointer("/endedAt"))
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

/// What a reload would fold for this session, reduced to the facts the guards check.
async fn failure_record(j: &mut Journey, client: &RpcClient, ws: &str, id: &str) -> Value {
    let sessions = j
        .call(
            client,
            Request::ListSessions {
                workstream_id: ws.to_string(),
            },
        )
        .await
        .expect("list_sessions");
    let row = sessions
        .as_array()
        .and_then(|rows| rows.iter().find(|r| text(r, "/id") == id))
        .cloned()
        .unwrap_or(Value::Null);
    let state = text(&row, "/state");
    let ended = row.pointer("/endedAt").is_some_and(|v| !v.is_null());
    let summary = row.pointer("/exitSummary").is_some_and(|v| !v.is_null());
    let log = j
        .call(
            client,
            Request::GetSessionLog {
                session_id: id.to_string(),
            },
        )
        .await
        .expect("get_session_log");
    let rows = log.as_array().cloned().unwrap_or_default();
    let exits: Vec<&Value> = rows
        .iter()
        .filter(|r| text(r, "/event/kind") == "exited")
        .collect();
    let failed = exits.iter().any(|r| {
        matches!(
            text(r, "/event/outcome").as_str(),
            "spawn_failed" | "crashed"
        ) && !text(r, "/event/stderrTail").is_empty()
    });
    let prompt = rows.iter().any(|r| {
        text(r, "/event/kind") == "userMessage" && text(r, "/event/text").contains("E2E_")
    });
    json!({ "state": state, "ended": ended, "summary": summary, "exits": exits.len(), "failed": failed, "prompt": prompt })
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

/// User turns claude read, not the title request that also reaches the stub.
fn turns_with(harness_log: &std::path::Path, prompt: &str) -> usize {
    std::fs::read_to_string(harness_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(r#""type":"user""#) && l.contains(prompt))
        .count()
}

fn watch(session_id: &str, rev: u64) -> Request {
    Request::WatchSessions {
        sessions: vec![WatchedSession {
            session_id: session_id.into(),
            rev,
        }],
    }
}

fn list_turns(session_id: &str, cursor: LogCursor) -> Request {
    Request::ListSessionTurns {
        session_id: session_id.into(),
        cursor,
        limit: 100,
    }
}

fn rev_of(value: &Value) -> u64 {
    value.pointer("/rev").and_then(Value::as_u64).unwrap_or(0)
}

fn event_kinds(events: &[SessionEvent]) -> Vec<&'static str> {
    events.iter().map(SessionEvent::kind).collect()
}

fn ids(value: &Value, pointer: &str) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(|i| text(i, pointer)).collect())
        .unwrap_or_default()
}
