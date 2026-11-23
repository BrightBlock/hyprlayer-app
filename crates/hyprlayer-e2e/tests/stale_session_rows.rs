//! A session row outlives its harness process: whether the server stops cleanly
//! or dies, the next server settles the rows the last one left live, wakes the
//! sessions it cut off mid-turn or with a background task open, and leaves a
//! cleanly idle or ended session asleep. A session that never answers its wake
//! is woken at most three times in a row. A restart before the wakes run keeps
//! them for the next start, which runs at most two at once.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const WAKE_HEADER: &str = "[from hyprlayer]";
const WAKE_PASS: &str = "session(s) the last server stopped";
const WAKE_CAPPED: &str = "restart wakes never finished a turn";
const WAKES_LEFT: &str = "wake(s) an earlier start never ran";
const NO_CONVERSATION: &str = "never started a conversation to resume";
const TASK: &str = "Watch the build";

/// Logs each launch's argv and every stdin line under `stub-sessions/<session id>/`. A turn
/// carrying `ASK_PERMISSION` asks and waits for the answer; one carrying `BACKGROUND` starts a
/// background Bash task; with `hang` in the session's directory no turn is ever answered. With
/// `noinit` in `stub-sessions/` a launch never reports its init or answers; with `slow` there, a
/// session's launch logs `start` and `init` around a 1 s wait to `stub-sessions/launches.log`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
argv="$*"
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id|--resume) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
stubs="$(dirname "$0")/../stub-sessions"
dir="$stubs/$sid"
mkdir -p "$dir"
printf '%s\n' "$(printf '%s' "$argv" | tr '\n' ' ')" >> "$dir/argv.log"
noinit=""
[ -f "$stubs/noinit" ] && noinit=1
if [ -n "$sid" ] && [ -f "$stubs/slow" ]; then
  printf 'start %s\n' "$sid" >> "$stubs/launches.log"
  sleep 1
  printf 'init %s\n' "$sid" >> "$stubs/launches.log"
fi
[ -z "$noinit" ] && printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$dir/stdin.log"
  case "$line" in *'"type":"user"'*) ;; *) continue ;; esac
  [ -f "$dir/hang" ] || [ -n "$noinit" ] && continue
  turn=$((turn + 1))
  case "$line" in
    *ASK_PERMISSION*)
      printf '{"type":"control_request","request_id":"req-%s","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}\n' "$turn"
      IFS= read -r response || break
      printf '%s\n' "$response" >> "$dir/stdin.log"
      ;;
    *BACKGROUND*)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu-bg-%s","name":"Bash","input":{"command":"make watch","run_in_background":true,"description":"Watch the build"}}]},"session_id":"%s"}\n' "$turn" "$sid"
      printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu-bg-%s","content":"Command running in background with ID: bg%s."}]},"session_id":"%s"}\n' "$turn" "$turn" "$sid"
      ;;
  esac
  printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":%s,"total_cost_usd":0,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_session_rows() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let stubs = sandbox.root.join("stub-sessions");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("stale_session_rows", &sandbox);
    let (server, client) = boot(&mut j, &sandbox).await;

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Stale rows").await;
    let ws = text(&view, "/item/id");

    j.step("park a session on an ask, open a background task in another, leave two idle, kill one");
    let (parked, mut parked_events) =
        start_session(&mut j, &client, &ws, "ASK_PERMISSION first").await;
    j.alias(&parked, "$PARKED");
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
    let (background, mut background_events) =
        start_session(&mut j, &client, &ws, "BACKGROUND watch the build").await;
    j.alias(&background, "$BACKGROUND");
    let started = recv_until(&mut background_events, is_turn_done).await;
    let opened = started
        .iter()
        .any(|e| matches!(e, SessionEvent::ToolUse { input, .. } if input.contains(TASK)));
    j.guard(
        "a turn starts a background task and ends",
        opened && started.last().is_some_and(is_turn_done),
        json!({ "opened": opened, "turnDone": started.last().is_some_and(is_turn_done) }),
    );
    let (idle, mut idle_events) = start_session(&mut j, &client, &ws, "hello").await;
    j.alias(&idle, "$IDLE");
    recv_until(&mut idle_events, is_turn_done).await;
    let (looping, mut loop_events) = start_session(&mut j, &client, &ws, "hello").await;
    j.alias(&looping, "$LOOP");
    recv_until(&mut loop_events, is_turn_done).await;
    std::fs::write(stubs.join(&looping).join("hang"), "").expect("hang the loop stub");
    send(&mut j, &client, &looping, "keep going").await;
    let hung = eventually(IO, || user_lines(&stubs, &looping) == 2).await;
    j.guard(
        "a stub that never answers holds a turn open",
        hung,
        json!({ "hung": hung }),
    );
    let (ended, mut ended_events) = start_session(&mut j, &client, &ws, "hello").await;
    j.alias(&ended, "$ENDED");
    recv_until(&mut ended_events, is_turn_done).await;
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
        state_of(&states, &parked) == "awaiting_permission"
            && state_of(&states, &ended) == "killed",
        states.clone(),
    );

    let names = [
        ("parked", parked.as_str()),
        ("background", background.as_str()),
        ("idle", idle.as_str()),
        ("loop", looping.as_str()),
        ("ended", ended.as_str()),
    ];

    j.step("restart after a clean shutdown");
    drop((
        parked_events,
        background_events,
        idle_events,
        loop_events,
        ended_events,
    ));
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the server stops",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );
    let (server, client) = boot(&mut j, &sandbox).await;
    let passed = wake_passes(&sandbox, 1).await;
    let record = stub_record_when(&stubs, &names, |r| {
        ["parked", "background", "loop"]
            .iter()
            .all(|name| r[name]["wakes"] == 1)
    })
    .await;
    let woken = ["parked", "background", "loop"];
    j.guard(
        "the sessions cut off mid-turn or with a task open are resumed, each woken once",
        passed
            && woken.iter().all(|name| {
                record[name]["launches"] == 2
                    && record[name]["resumed"] == true
                    && record[name]["wakes"] == 1
            }),
        record.clone(),
    );
    j.guard(
        "only the background session's wake names its task",
        record["background"]["namesTask"] == true
            && record["parked"]["namesTask"] == false
            && record["loop"]["namesTask"] == false,
        record.clone(),
    );
    j.guard(
        "the clean idle and the ended sessions stay asleep",
        ["idle", "ended"]
            .iter()
            .all(|name| record[name]["launches"] == 1 && record[name]["wakes"] == 0),
        record.clone(),
    );
    let live = live_names(&mut j, &client, &names).await;
    j.guard(
        "only the woken sessions are live",
        live == ["background", "loop", "parked"],
        json!({ "live": live }),
    );
    let states = session_states(&mut j, &client, &ws).await;
    j.guard(
        "the ended row keeps its outcome",
        state_of(&states, &ended) == "killed",
        json!({ "ended": state_of(&states, &ended) }),
    );

    j.step("park the woken session again and restart after a crash");
    let (_, _, _, mut resumed) = client
        .resume_session_stream(parked.clone(), None, None, None, None)
        .await
        .expect("resume_session");
    send(&mut j, &client, &parked, "ASK_PERMISSION again").await;
    let asked = recv_until(&mut resumed, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    let states = session_states(&mut j, &client, &ws).await;
    j.guard(
        "the woken session parks on a new ask",
        asked
            .iter()
            .any(|e| matches!(e, SessionEvent::PermissionAsk { .. }))
            && state_of(&states, &parked) == "awaiting_permission",
        json!({ "parked": state_of(&states, &parked) }),
    );
    drop(resumed);
    let gone = crash(server).await;
    j.guard(
        "the crashed server stops listening",
        gone,
        json!({ "gone": gone }),
    );
    let (server, _client) = boot(&mut j, &sandbox).await;
    let passed = wake_passes(&sandbox, 2).await;
    let record = stub_record_when(&stubs, &names, |r| {
        r["parked"]["wakes"] == 2 && r["loop"]["wakes"] == 2
    })
    .await;
    j.guard(
        "the mid-turn sessions get one more wake each",
        passed
            && record["parked"]["wakes"] == 2
            && record["parked"]["launches"] == 3
            && record["loop"]["wakes"] == 2,
        record.clone(),
    );
    j.guard(
        "a task from before the last relaunch and a finished turn wake nothing",
        record["background"]["wakes"] == 1
            && record["background"]["launches"] == 2
            && record["idle"]["launches"] == 1
            && record["ended"]["launches"] == 1,
        record.clone(),
    );

    j.step("a session that never answers its wake is woken three times at most, stored or not");
    let gone = crash(server).await;
    let (server, _client) = boot(&mut j, &sandbox).await;
    let third = wake_passes(&sandbox, 3).await;
    let wakes_after_third = stub_record_when(&stubs, &names, |r| r["loop"]["wakes"] == 3).await
        ["loop"]["wakes"]
        .clone();
    let gone = gone && crash(server).await;
    // As if each restart had landed before the actor stored its wake's echo.
    let unstored = rusqlite::Connection::open(sandbox.app_data.join("hyprlayer.db"))
        .expect("open the profile db")
        .execute(
            "DELETE FROM session_events
              WHERE session_id = ?1 AND kind = 'userMessage' AND payload LIKE '%hyprlayer:restart%'",
            rusqlite::params![looping],
        )
        .expect("drop the stored wakes");
    let (server, client) = boot(&mut j, &sandbox).await;
    let fourth = wake_passes(&sandbox, 4).await;
    let record = stub_record(&stubs, &names);
    let capped = sandbox
        .server_log()
        .lines()
        .any(|line| line.contains(&looping) && line.contains(WAKE_CAPPED));
    j.guard(
        "the third restart wakes it, the fourth logs the cap instead",
        gone && third
            && fourth
            && wakes_after_third == 3
            && unstored == 3
            && record["loop"]["wakes"] == 3
            && record["loop"]["launches"] == 4
            && capped,
        json!({
            "wakesAfterThird": wakes_after_third,
            "wakeRowsUnstored": unstored,
            "wakesAfterFourth": record["loop"]["wakes"],
            "launches": record["loop"]["launches"],
            "capLogged": capped,
        }),
    );

    j.step("a restart before the wakes run keeps them, and the next start runs two at once");
    std::fs::write(stubs.join("noinit"), "").expect("stop the stub's inits");
    let (unstarted, unstarted_events) = start_session(&mut j, &client, &ws, "hello").await;
    j.alias(&unstarted, "$UNSTARTED");
    let sent = eventually(IO, || user_lines(&stubs, &unstarted) == 1).await;
    std::fs::remove_file(stubs.join("noinit")).expect("restore the stub's inits");
    let mut cut_off = Vec::new();
    let mut streams = vec![unstarted_events];
    for alias in ["$FIRST", "$SECOND", "$THIRD"] {
        let (id, mut events) = start_session(&mut j, &client, &ws, "ASK_PERMISSION first").await;
        j.alias(&id, alias);
        recv_until(&mut events, |e| {
            matches!(e, SessionEvent::PermissionAsk { .. })
        })
        .await;
        cut_off.push(id);
        streams.push(events);
    }
    drop(streams);
    let gone = crash(server).await;
    let windowed = Server::start(&sandbox).await;
    let gone = gone && crash(windowed).await;
    let windowed_pass = pass_count(&sandbox) > 4;
    std::fs::write(stubs.join("slow"), "").expect("slow the stub's inits");
    let (server, client) = boot(&mut j, &sandbox).await;
    let passed = wake_passes(&sandbox, 5).await;
    let woken: Vec<(&str, &str)> = ["first", "second", "third"]
        .into_iter()
        .zip(cut_off.iter().map(String::as_str))
        .collect();
    let record = stub_record_when(&stubs, &woken, |r| {
        woken.iter().all(|(name, _)| r[name]["wakes"] == 1)
    })
    .await;
    let once = woken.iter().all(|(name, _)| {
        record[name]["launches"] == 2
            && record[name]["resumed"] == true
            && record[name]["wakes"] == 1
    });
    let log = sandbox.server_log();
    j.guard(
        "a start killed before its wakes run leaves them to the next, which wakes each once",
        gone && !windowed_pass && passed && once && log.contains(WAKES_LEFT),
        json!({
            "gone": gone,
            "windowedPass": windowed_pass,
            "passed": passed,
            "wokenOnce": once,
            "leftoverLogged": log.contains(WAKES_LEFT),
        }),
    );
    let launches = std::fs::read_to_string(stubs.join("launches.log")).unwrap_or_default();
    let (mut open, mut most, mut starts) = (0, 0, 0);
    for line in launches.lines() {
        if line.starts_with("start ") {
            open += 1;
            starts += 1;
            most = most.max(open);
        } else if line.starts_with("init ") {
            open -= 1;
        }
    }
    j.guard(
        "at most two wakes start at once",
        starts == 3 && most <= 2,
        json!({ "threeStarted": starts == 3, "atMostTwo": most <= 2 }),
    );
    let skipped = log
        .lines()
        .any(|line| line.contains(&unstarted) && line.contains(NO_CONVERSATION));
    let launched_once = stub_lines(&stubs, &unstarted, "argv.log").len() == 1;
    j.guard(
        "a session cut off before its harness started is not relaunched as a new conversation",
        sent && skipped && launched_once,
        json!({ "sent": sent, "skipLogged": skipped, "launchedOnce": launched_once }),
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

async fn boot(j: &mut Journey, sandbox: &Sandbox) -> (Server, Arc<RpcClient>) {
    let server = Server::start(sandbox).await;
    let client = server.client();
    j.call(
        &client,
        Request::SetEnvOverlay {
            vars: BTreeMap::from([("LANG".to_string(), "C.UTF-8".to_string())]),
        },
    )
    .await
    .expect("set_env_overlay");
    (server, client)
}

async fn crash(server: Server) -> bool {
    let socket = server.socket().to_path_buf();
    // kill_on_drop: SIGKILL, so no exit event is ever written.
    drop(server);
    let deadline = Instant::now() + IO;
    loop {
        if RpcClient::connect_socket(&socket).await.is_err() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wake_passes(sandbox: &Sandbox, n: usize) -> bool {
    eventually(IO, || pass_count(sandbox) >= n).await
}

fn pass_count(sandbox: &Sandbox) -> usize {
    sandbox
        .server_log()
        .lines()
        .filter(|line| line.contains(WAKE_PASS))
        .count()
}

fn stub_lines(stubs: &Path, id: &str, file: &str) -> Vec<String> {
    std::fs::read_to_string(stubs.join(id).join(file))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn user_lines(stubs: &Path, id: &str) -> usize {
    stub_lines(stubs, id, "stdin.log")
        .iter()
        .filter(|line| line.contains(r#""type":"user""#))
        .count()
}

fn stub_record(stubs: &Path, names: &[(&str, &str)]) -> Value {
    let map = names
        .iter()
        .map(|(name, id)| {
            let argv = stub_lines(stubs, id, "argv.log");
            let wakes: Vec<String> = stub_lines(stubs, id, "stdin.log")
                .into_iter()
                .filter(|line| line.contains(WAKE_HEADER))
                .collect();
            let record = json!({
                "launches": argv.len(),
                "resumed": argv.last().is_some_and(|a| a.contains("--resume")),
                "wakes": wakes.len(),
                "namesTask": wakes.first().is_some_and(|w| w.contains(TASK)),
            });
            (name.to_string(), record)
        })
        .collect();
    Value::Object(map)
}

async fn stub_record_when(
    stubs: &Path,
    names: &[(&str, &str)],
    ready: impl Fn(&Value) -> bool,
) -> Value {
    let mut record = stub_record(stubs, names);
    eventually(IO, || {
        record = stub_record(stubs, names);
        ready(&record)
    })
    .await;
    record
}

async fn live_names(j: &mut Journey, client: &RpcClient, names: &[(&str, &str)]) -> Vec<String> {
    let active = j
        .call(client, Request::ListActiveSessions {})
        .await
        .expect("list_active_sessions");
    let ids: Vec<String> = active
        .as_array()
        .map(|rows| rows.iter().map(|r| text(r, "/session/id")).collect())
        .unwrap_or_default();
    let mut live: Vec<String> = names
        .iter()
        .filter(|(_, id)| ids.iter().any(|live| live == id))
        .map(|(name, _)| name.to_string())
        .collect();
    live.sort();
    live
}

fn is_turn_done(event: &SessionEvent) -> bool {
    matches!(event, SessionEvent::TurnDone { .. })
}

async fn send(j: &mut Journey, client: &RpcClient, session_id: &str, text: &str) {
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
