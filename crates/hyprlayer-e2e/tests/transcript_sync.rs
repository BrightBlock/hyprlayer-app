//! Several clients, one session: announced clients hear each other's commits,
//! authors and viewers, a send behind another client's turns is refused before
//! the harness sees it, a reconnect catches up gaplessly, and a client that
//! never announced keeps the released wire.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_proto::{StaleTranscript, WireError};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::events::{TARGETED_EVENTS, TRANSCRIPT_SYNC};
use hyprlayer_server::rpc::WatchedSession;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};

const IO: Duration = Duration::from_secs(20);
const MAX_PAGES: usize = 20;

/// `session_lifecycle.rs`'s script: numbered replies, an `ASK_PERMISSION`
/// hold, and every line it is handed appended to `<sandbox>/harness.log`.
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

/// A connection with its own runtime and thread, so it can really leave:
/// dropping an `RpcClient` alone leaves its reader holding the socket open.
struct Departing {
    client: Arc<RpcClient>,
    leave: std::sync::mpsc::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl Departing {
    fn connect(socket: PathBuf) -> Self {
        let (client_tx, client_rx) = std::sync::mpsc::channel();
        let (leave, left) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("build the departing client's runtime");
            let client = runtime
                .block_on(RpcClient::connect_socket(&socket))
                .expect("connect the departing client");
            client_tx.send(client).expect("hand the client back");
            let _ = left.recv();
            runtime.shutdown_timeout(Duration::from_secs(5));
        });
        let client = client_rx.recv().expect("the departing client connects");
        Self {
            client,
            leave,
            thread,
        }
    }

    async fn leave(self) {
        let Self {
            client,
            leave,
            thread,
        } = self;
        drop(client);
        let _ = leave.send(());
        tokio::task::spawn_blocking(move || thread.join().expect("the departing client's thread"))
            .await
            .expect("join the departing client");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transcript_sync() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let harness_log = sandbox.root.join("harness.log");
    let harness_saw = |needle: &str| {
        std::fs::read_to_string(&harness_log)
            .unwrap_or_default()
            .contains(needle)
    };
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("transcript_sync", &sandbox);
    let server = Server::start(&sandbox).await;
    let a = server.client();
    let socket = server.socket().to_path_buf();
    let b = tokio::task::spawn_blocking(move || Departing::connect(socket))
        .await
        .expect("connect B");
    let c = server.connect().await;
    let mut a_events = a.take_events().expect("A's runtime events");
    let mut b_events = b.client.take_events().expect("B's runtime events");
    let mut c_events = c.take_events().expect("C's runtime events");

    let project = onboard_and_import(&mut j, &a, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &a, &project, "Transcript sync").await;
    let ws = text(&view, "/item/id");

    j.step("clients announce");
    let a_announced = j.call(&a, announce("client-a", "device-a")).await;
    let b_announced = j.call(&b.client, announce("client-b", "device-b")).await;
    let syncs = |reply: &Result<Value, WireError>| {
        reply.as_ref().is_ok_and(|v| {
            v.pointer("/features")
                .and_then(Value::as_array)
                .is_some_and(|f| f.iter().any(|f| f == TRANSCRIPT_SYNC))
        })
    };
    j.guard(
        "the server reports transcript sync",
        syncs(&a_announced) && syncs(&b_announced),
        json!({ "a": syncs(&a_announced), "b": syncs(&b_announced) }),
    );

    j.step("A starts a session with metadata");
    let (response, _, mut a_stream) = a
        .open_session_stream(StreamRequest::StartStageSession {
            workstream_id: ws.clone(),
            stage: Stage::General,
            input: "E2E_PROMPT_ONE".into(),
            attached_docs: vec![],
            opts: StreamStartOpts {
                harness: Some(Harness::Claude),
                client_message_id: Some("cmid-a1".into()),
                ..Default::default()
            },
            echo_user_messages: true,
            with_meta: true,
        })
        .await
        .expect("start_stage_session");
    let StreamResponse::StartStageSession { session, .. } = response else {
        panic!("start_stage_session answered with another variant");
    };
    let sid = session.id.clone();
    j.alias(&sid, "$SESSION");
    j.call(
        &a,
        Request::LaunchAndConverse {
            session_id: sid.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let first = drain_frames(&mut a_stream, 1).await;
    let user = item_where(&first, |e| matches!(e, SessionEvent::UserMessage { .. }));
    j.guard(
        "A's user message carries A and rev 1",
        user.is_some_and(|i| i.rev == 1 && i.seq.is_some() && author_id(i) == Some("client-a")),
        json!({
            "rev": user.map(|i| i.rev),
            "device": user.and_then(author_device),
            "hasSeq": user.is_some_and(|i| i.seq.is_some()),
        }),
    );
    let reply = item_where(&first, |e| matches!(e, SessionEvent::AssistantText { .. }));
    j.guard(
        "the reply commits rev 2 under A",
        reply.is_some_and(|i| i.rev == 2 && author_id(i) == Some("client-a")),
        json!({ "rev": reply.map(|i| i.rev), "device": reply.and_then(author_device) }),
    );
    let partial = item_where(&first, |e| matches!(e, SessionEvent::PartialText { .. }));
    j.guard(
        "partial text has no seq",
        partial.is_some_and(|i| i.seq.is_none()),
        json!({ "partial": partial.is_some(), "hasSeq": partial.is_some_and(|i| i.seq.is_some()) }),
    );

    j.step("both watch the session");
    j.call(&a, watch(&sid, 0)).await.expect("A watches");
    let b_watch = j.call(&b.client, watch(&sid, 0)).await.expect("B watches");
    let state = &b_watch[0];
    j.guard(
        "watch answers the current state and its viewers",
        state["rev"] == 2
            && state["resetRev"] == 0
            && state["working"] == false
            && state["inFlight"] == 0
            && devices(&state["viewers"]) == ["device-a", "device-b"],
        session_state(state),
    );
    let seen = events_until(&mut b_events, IO, |e| {
        viewers_of(e, &sid).is_some_and(|v| v.len() == 2)
    })
    .await;
    let b_viewers = seen.iter().rev().find_map(|e| viewers_of(e, &sid));
    j.guard(
        "B learns both devices view it",
        b_viewers.as_deref() == Some(&["device-a".to_string(), "device-b".to_string()][..]),
        json!({ "viewers": b_viewers }),
    );

    j.step("B follows A's next turn");
    let turns = j
        .call(&b.client, list_turns(&sid, LogCursor::Empty))
        .await
        .expect("B reads the log");
    let mut b_cursor = next_cursor_of(&turns);
    let (_, _, mut b_stream) = b
        .client
        .open_session_stream(StreamRequest::ResumeSession {
            session_id: sid.clone(),
            model: None,
            effort: None,
            permission_mode: None,
            cursor: Some(b_cursor),
            echo_user_messages: true,
            with_meta: true,
        })
        .await
        .expect("B resumes");
    let (sent, a_frames, b_frames) = tokio::join!(
        j.call(&a, send(&sid, "E2E_PROMPT_TWO", Some("cmid-a2"), Some(2))),
        drain_frames(&mut a_stream, 1),
        drain_frames(&mut b_stream, 1),
    );
    sent.expect("A sends its second turn");
    let b_user = item_where(&b_frames, |e| matches!(e, SessionEvent::UserMessage { .. }));
    let b_reply = item_where(&b_frames, |e| {
        matches!(e, SessionEvent::AssistantText { .. })
    });
    j.guard(
        "B's stream carries A's turn attributed to A",
        b_user.is_some_and(|i| i.rev == 3 && author_id(i) == Some("client-a"))
            && b_reply.is_some_and(|i| i.rev == 4 && author_id(i) == Some("client-a")),
        json!({
            "user": { "rev": b_user.map(|i| i.rev), "device": b_user.and_then(author_device) },
            "reply": { "rev": b_reply.map(|i| i.rev), "device": b_reply.and_then(author_device) },
        }),
    );
    let seen = events_until(&mut b_events, IO, |e| {
        matches!(e, RuntimeEvent::SessionRevised { session_id, rev: 4, turns, working: false, .. }
            if *session_id == sid && turns.is_empty())
    })
    .await;
    let revised: Vec<&RuntimeEvent> = seen
        .iter()
        .filter(
            |e| matches!(e, RuntimeEvent::SessionRevised { session_id, .. } if *session_id == sid),
        )
        .collect();
    let commits: Vec<Value> = revised
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::SessionRevised {
                rev,
                turns,
                author,
                working,
                ..
            } if !turns.is_empty() => Some(json!({
                "rev": rev,
                "turns": turns.iter().map(|t| json!(t.kind)).collect::<Vec<_>>(),
                "working": working,
                "device": author.as_ref().map(|a| a.device.clone()),
            })),
            _ => None,
        })
        .collect();
    let expected_commits = json!([
        { "rev": 3, "turns": ["user"], "working": true, "device": "device-a" },
        { "rev": 4, "turns": ["assistant"], "working": true, "device": "device-a" },
    ]);
    j.guard(
        "B hears each commit with its author",
        Value::Array(commits.clone()) == expected_commits,
        json!({ "commits": commits }),
    );
    let turn_end = revised.iter().any(|e| {
        matches!(e, RuntimeEvent::SessionRevised { rev: 4, turns, working: false, .. } if turns.is_empty())
    });
    j.guard(
        "the turn's end is announced",
        turn_end,
        json!({ "rev": 4, "working": false, "seen": turn_end }),
    );
    let a_reply_at = received_at(&a_frames, |e| {
        matches!(e, SessionEvent::AssistantText { .. })
    });
    let b_reply_at = received_at(&b_frames, |e| {
        matches!(e, SessionEvent::AssistantText { .. })
    });
    let within = match (a_reply_at, b_reply_at) {
        (Some(x), Some(y)) => x.max(y).duration_since(x.min(y)) < Duration::from_secs(1),
        _ => false,
    };
    j.guard(
        "B receives A's committed reply within 1s of A",
        within,
        json!({ "withinOneSecond": within }),
    );

    j.step("the stored message keeps its author");
    let log = j
        .call(
            &a,
            Request::GetSessionLog {
                session_id: sid.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let last_user = user_messages(&log).last().cloned().cloned();
    let author = last_user.as_ref().map(|m| m["author"].clone());
    j.guard(
        "the last user message is stored with A as its author",
        author == Some(json!({ "clientId": "client-a", "device": "device-a" })),
        json!({ "author": author }),
    );

    j.step("a client's own sends never make it stale");
    let self_one = j.call(&a, send(&sid, "E2E_SELF_ONE", None, Some(4))).await;
    let self_two = j.call(&a, send(&sid, "E2E_SELF_TWO", None, Some(4))).await;
    drain_frames(&mut a_stream, 2).await;
    j.guard(
        "a client's back-to-back sends are both accepted",
        self_one.is_ok() && self_two.is_ok(),
        json!({ "first": err_code(&self_one), "second": err_code(&self_two) }),
    );
    let early = j
        .call(&b.client, send(&sid, "E2E_B_EARLY", None, Some(4)))
        .await;
    stale_guard(
        &mut j,
        "another client's older base is refused",
        &early,
        (8, 8, 4),
    );
    j.guard(
        "B's text never reached the harness",
        !harness_saw("E2E_B_EARLY"),
        json!({ "harnessSawIt": harness_saw("E2E_B_EARLY") }),
    );
    let (rev, cursor) = pull(&mut j, &b.client, &sid, b_cursor).await;
    b_cursor = cursor;
    j.guard("B's pull reaches rev 8", rev == 8, json!({ "rev": rev }));

    j.step("an app follow-up belongs to the client whose turn triggered it");
    let (ship, _, _, mut ship_events) = a
        .start_stage_session_stream(
            ws.clone(),
            Stage::Ship,
            "E2E_SHIP".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                client_message_id: Some("cmid-ship".into()),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session (ship)");
    let ship_id = ship.id.clone();
    j.alias(&ship_id, "$SHIP_SESSION");
    j.call(
        &a,
        Request::LaunchAndConverse {
            session_id: ship_id.clone(),
        },
    )
    .await
    .expect("launch_and_converse (ship)");
    drain_turns(&mut ship_events, 2).await;
    let log = j
        .call(
            &a,
            Request::GetSessionLog {
                session_id: ship_id.clone(),
            },
        )
        .await
        .expect("get_session_log (ship)");
    let followup = user_messages(&log).get(1).cloned().cloned();
    let minted = followup.as_ref().is_some_and(|m| {
        m["clientMessageId"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && id != "cmid-ship")
    });
    let followup_author = followup
        .as_ref()
        .and_then(|m| m.pointer("/author/clientId").and_then(Value::as_str))
        .map(str::to_string);
    j.guard(
        "the follow-up is attributed to A",
        minted && followup_author.as_deref() == Some("client-a"),
        json!({ "serverMintedNonce": minted, "authorClientId": followup_author }),
    );
    let foreign = j
        .call(&b.client, send(&ship_id, "E2E_B_SHIP", None, Some(2)))
        .await;
    stale_guard(
        &mut j,
        "the follow-up is foreign to another client",
        &foreign,
        (4, 4, 2),
    );
    let own = j
        .call(&a, send(&ship_id, "E2E_SHIP_SELF", None, Some(2)))
        .await;
    drain_turns(&mut ship_events, 1).await;
    j.guard(
        "the follow-up never makes its own client stale",
        own.is_ok(),
        json!({ "error": err_code(&own) }),
    );

    j.step("both send at once");
    j.call(
        &a,
        send(&sid, "ASK_PERMISSION E2E_PROMPT_THREE", None, Some(8)),
    )
    .await
    .expect("A sends a turn that asks");
    let asked = recv_frames_until(&mut a_stream, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    let request_id = asked
        .iter()
        .find_map(|(_, f)| match event_of(f) {
            SessionEvent::PermissionAsk { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
        .expect("the harness asks for permission");
    let racing = j
        .call(&b.client, send(&sid, "E2E_PROMPT_B", None, Some(8)))
        .await;
    stale_guard(
        &mut j,
        "the second send is refused with stale_transcript",
        &racing,
        (9, 9, 1),
    );
    j.guard(
        "B's text never reached the harness",
        !harness_saw("E2E_PROMPT_B"),
        json!({ "harnessSawIt": harness_saw("E2E_PROMPT_B") }),
    );
    j.call(
        &a,
        Request::RespondPermission {
            session_id: sid.clone(),
            request_id,
            allow: true,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
    drain_frames(&mut a_stream, 1).await;
    let (rev, cursor) = pull(&mut j, &b.client, &sid, b_cursor).await;
    b_cursor = cursor;
    j.guard("B's pull reaches rev 10", rev == 10, json!({ "rev": rev }));
    let resent = j
        .call(&b.client, send(&sid, "E2E_PROMPT_B", None, Some(10)))
        .await;
    let resend_frames = drain_frames(&mut a_stream, 1).await;
    let b_turn = item_where(
        &resend_frames,
        |e| matches!(e, SessionEvent::UserMessage { text, .. } if text.contains("E2E_PROMPT_B")),
    );
    j.guard(
        "the resend after pulling is accepted and attributed to B",
        resent.is_ok() && b_turn.is_some_and(|i| author_id(i) == Some("client-b")),
        json!({
            "error": err_code(&resent),
            "rev": b_turn.map(|i| i.rev),
            "device": b_turn.and_then(author_device),
        }),
    );

    j.step("B leaves and catches up on reconnect");
    drop(b_stream);
    events_until(&mut a_events, Duration::from_millis(200), |_| false).await;
    b.leave().await;
    let seen = events_until(&mut a_events, IO, |e| viewers_of(e, &sid).is_some()).await;
    let a_viewers = seen.iter().find_map(|e| viewers_of(e, &sid));
    j.guard(
        "viewers shrink when a client leaves",
        a_viewers.as_deref() == Some(&["device-a".to_string()][..]),
        json!({ "viewers": a_viewers }),
    );
    j.call(&a, send(&sid, "E2E_PROMPT_OFFLINE", None, Some(12)))
        .await
        .expect("A sends while B is away");
    drain_frames(&mut a_stream, 1).await;
    let b2 = server.connect().await;
    j.call(&b2, announce("client-b", "device-b"))
        .await
        .expect("B announces again");
    let b2_watch = j.call(&b2, watch(&sid, 12)).await.expect("B watches again");
    let state = &b2_watch[0];
    j.guard(
        "the reconnect is told the current state and viewers",
        state["rev"] == 14
            && state["resetRev"] == 0
            && state["inFlight"] == 0
            && devices(&state["viewers"]) == ["device-a", "device-b"],
        session_state(state),
    );
    let LogCursor::After { seq: from, .. } = b_cursor else {
        panic!("B's cursor names a row");
    };
    let missed = read_all(&mut j, &b2, &sid, b_cursor).await;
    let mut seqs = vec![from];
    seqs.extend(missed.iter().filter_map(|r| r["seq"].as_i64()));
    let increasing = seqs.windows(2).all(|w| w[0] < w[1]);
    let has_own_turn = missed.iter().any(|r| {
        r.pointer("/event/kind") == Some(&json!("userMessage"))
            && r.pointer("/event/text")
                .and_then(Value::as_str)
                .is_some_and(|t| t.contains("E2E_PROMPT_B"))
            && r.pointer("/event/author/clientId") == Some(&json!("client-b"))
    });
    let offline_at = missed.iter().position(|r| {
        r.pointer("/event/text")
            .and_then(Value::as_str)
            .is_some_and(|t| t.contains("E2E_PROMPT_OFFLINE"))
    });
    let has_offline_reply = offline_at.is_some_and(|at| {
        missed[at..]
            .iter()
            .any(|r| r.pointer("/event/kind") == Some(&json!("assistantText")))
    });
    j.guard(
        "every missed row once, in order",
        increasing && has_own_turn && has_offline_reply,
        json!({
            "strictlyIncreasing": increasing,
            "hasOwnTurn": has_own_turn,
            "hasOfflineReply": has_offline_reply,
        }),
    );

    j.step("a released desktop's message is author-less");
    j.call(&c, send(&sid, "E2E_LEGACY", None, None))
        .await
        .expect("C sends the released desktop's shape");
    drain_frames(&mut a_stream, 1).await;
    let log = j
        .call(
            &a,
            Request::GetSessionLog {
                session_id: sid.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let legacy = user_messages(&log)
        .into_iter()
        .find(|m| m["text"].as_str().is_some_and(|t| t.contains("E2E_LEGACY")))
        .cloned();
    let legacy_author = legacy.as_ref().map(|m| m["author"].clone());
    j.guard(
        "no author is inferred for a nonce-less message",
        legacy_author == Some(Value::Null),
        json!({ "author": legacy_author }),
    );
    let behind = j.call(&a, send(&sid, "E2E_A_BEHIND", None, Some(14))).await;
    stale_guard(
        &mut j,
        "it is foreign to every announced client",
        &behind,
        (16, 16, 2),
    );

    j.step("an unannounced client sees none of it");
    let seen = events_until(&mut c_events, Duration::from_millis(500), |_| false).await;
    let saw_lifecycle = seen.iter().any(
        |e| matches!(e, RuntimeEvent::SessionLifecycle { session_id, .. } if *session_id == sid),
    );
    let saw_targeted = seen.iter().any(|e| {
        matches!(
            e,
            RuntimeEvent::SessionRevised { .. }
                | RuntimeEvent::SessionReset { .. }
                | RuntimeEvent::SessionViewers { .. }
        )
    });
    let (_, _, _, mut c_stream) = c
        .resume_session_stream(sid.clone(), None, None, None, Some(LogCursor::Empty))
        .await
        .expect("C resumes on the typed wrapper");
    let first_bare = tokio::time::timeout(IO, c_stream.recv())
        .await
        .ok()
        .flatten();
    j.guard(
        "the released-client path is unchanged",
        saw_lifecycle && !saw_targeted && first_bare.is_some(),
        json!({
            "sawLifecycle": saw_lifecycle,
            "sawTargeted": saw_targeted,
            "firstBareKind": first_bare.as_ref().map(SessionEvent::kind),
        }),
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

fn announce(client_id: &str, device: &str) -> Request {
    Request::AnnounceClient {
        client_id: client_id.into(),
        device: device.into(),
        capabilities: TARGETED_EVENTS.map(str::to_string).to_vec(),
    }
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

fn send(session_id: &str, text: &str, nonce: Option<&str>, base_rev: Option<u64>) -> Request {
    Request::SendUserMessage {
        session_id: session_id.into(),
        text: text.into(),
        attached_docs: vec![],
        prompt_name: None,
        client_message_id: nonce.map(str::to_string),
        base_rev,
    }
}

fn stale_guard(
    j: &mut Journey,
    guard: &str,
    reply: &Result<Value, WireError>,
    (rev, committed_rev, missing_turns): (u64, u64, u64),
) {
    let detail = reply.as_ref().err().and_then(|e| e.stale_transcript);
    j.guard(
        guard,
        err_code(reply).as_deref() == Some("stale_transcript")
            && detail
                == Some(StaleTranscript {
                    rev,
                    committed_rev,
                    missing_turns,
                }),
        json!({
            "code": err_code(reply),
            "rev": detail.map(|d| d.rev),
            "committedRev": detail.map(|d| d.committed_rev),
            "missingTurns": detail.map(|d| d.missing_turns),
        }),
    );
}

fn err_code(reply: &Result<Value, WireError>) -> Option<String> {
    reply.as_ref().err().map(|e| e.code.clone())
}

/// Pages from `cursor` to the end; returns the final rev and the new cursor.
async fn pull(
    j: &mut Journey,
    client: &RpcClient,
    session_id: &str,
    cursor: LogCursor,
) -> (u64, LogCursor) {
    let mut cursor = cursor;
    for _ in 0..MAX_PAGES {
        let page = j
            .call(client, list_turns(session_id, cursor))
            .await
            .expect("list_session_turns");
        cursor = next_cursor_of(&page);
        if page["remaining"] == 0 {
            return (page["rev"].as_u64().unwrap_or(0), cursor);
        }
    }
    panic!("the catch-up read never reported remaining: 0");
}

async fn read_all(
    j: &mut Journey,
    client: &RpcClient,
    session_id: &str,
    cursor: LogCursor,
) -> Vec<Value> {
    let mut cursor = cursor;
    let mut rows = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = j
            .call(client, list_turns(session_id, cursor))
            .await
            .expect("list_session_turns");
        rows.extend(page["entries"].as_array().cloned().unwrap_or_default());
        cursor = next_cursor_of(&page);
        if page["remaining"] == 0 {
            return rows;
        }
    }
    panic!("the catch-up read never reported remaining: 0");
}

fn next_cursor_of(turns: &Value) -> LogCursor {
    turns
        .pointer("/nextCursor")
        .cloned()
        .and_then(|c| serde_json::from_value(c).ok())
        .unwrap_or(LogCursor::Empty)
}

fn session_state(state: &Value) -> Value {
    json!({
        "rev": state["rev"],
        "resetRev": state["resetRev"],
        "working": state["working"],
        "inFlight": state["inFlight"],
        "viewers": devices(&state["viewers"]),
    })
}

fn devices(viewers: &Value) -> Vec<String> {
    viewers
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|c| c["device"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn viewers_of(event: &RuntimeEvent, sid: &str) -> Option<Vec<String>> {
    match event {
        RuntimeEvent::SessionViewers {
            session_id,
            viewers,
        } if session_id == sid => Some(viewers.iter().map(|v| v.device.clone()).collect()),
        _ => None,
    }
}

fn user_messages(log: &Value) -> Vec<&Value> {
    log.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| &r["event"])
                .filter(|e| e["kind"] == "userMessage")
                .collect()
        })
        .unwrap_or_default()
}

type Received = (Instant, SessionFrame);

fn event_of(frame: &SessionFrame) -> &SessionEvent {
    match frame {
        SessionFrame::Item(item) => &item.event,
        SessionFrame::Bare(event) => event,
    }
}

fn item_where(
    frames: &[Received],
    pick: impl Fn(&SessionEvent) -> bool,
) -> Option<&hyprlayer_core::models::SessionStreamItem> {
    frames.iter().find_map(|(_, f)| match f {
        SessionFrame::Item(item) if pick(&item.event) => Some(item),
        _ => None,
    })
}

fn received_at(frames: &[Received], pick: impl Fn(&SessionEvent) -> bool) -> Option<Instant> {
    frames
        .iter()
        .find(|(_, f)| pick(event_of(f)))
        .map(|(at, _)| *at)
}

fn author_id(item: &hyprlayer_core::models::SessionStreamItem) -> Option<&str> {
    item.author.as_ref().map(|a| a.client_id.as_str())
}

fn author_device(item: &hyprlayer_core::models::SessionStreamItem) -> Option<String> {
    item.author.as_ref().map(|a| a.device.clone())
}

/// Frames up to and including the `turns`-th `TurnDone`, stamped as they arrive.
async fn drain_frames(stream: &mut SessionEvents, turns: usize) -> Vec<Received> {
    let mut left = turns;
    recv_frames_until(stream, move |e| {
        if matches!(e, SessionEvent::TurnDone { .. }) {
            left -= 1;
        }
        left == 0
    })
    .await
}

async fn recv_frames_until(
    stream: &mut SessionEvents,
    mut stop: impl FnMut(&SessionEvent) -> bool,
) -> Vec<Received> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(frame) = stream.recv().await {
            let at = Instant::now();
            let done = stop(event_of(&frame));
            seen.push((at, frame));
            if done {
                return;
            }
        }
    })
    .await;
    seen
}

async fn drain_turns(events: &mut EventStream<SessionEvent>, turns: usize) {
    let mut left = turns;
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(event, SessionEvent::TurnDone { .. }) {
                left -= 1;
                if left == 0 {
                    return;
                }
            }
        }
    })
    .await;
}
