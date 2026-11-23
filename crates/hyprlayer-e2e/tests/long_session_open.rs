//! Opening a long session from another device: its newest rows arrive in one read, the
//! older ones page back from there, and a rewind refuses a cursor that reads back across it.
#![cfg(unix)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::models::SessionTurns;
use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Sandbox, Server};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const IO: Duration = Duration::from_secs(120);
const TURNS: usize = 10;
const STEPS: usize = 120;
/// What the desktop asks its first read for; the backfill asks for full pages.
const TAIL_ROWS: u32 = 100;
const PAGE_ROWS: u32 = 2000;
/// The server's page budget, plus room for one page's row envelopes.
const PAGE_BOUND: u64 = 256 * 1024 + 16 * 1024;
/// The phone link the first-open finding measured on.
const LINK_BYTES_PER_S: f64 = 256.0 * 1024.0;
const LINK_ROUND_TRIP_S: f64 = 0.2;

/// Replays `<sandbox>/turn.jsonl` for every message it reads, or `ask.jsonl` for one that says ASK_FIRST.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cfg="$(dirname "$0")/.."
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  script="$cfg/turn.jsonl"
  case "$line" in
    *ASK_FIRST*) script="$cfg/ask.jsonl" ;;
  esac
  sed -e "s/%SID%/$sid/g" -e "s/%TURN%/$turn/g" "$script"
done
"#;

/// `STEPS` file reads, each with a paragraph of prose and a 3 KB result.
fn long_turn() -> String {
    let mut out = String::new();
    for n in 0..STEPS {
        let id = format!("toolu_%TURN%_{n}");
        let prose = format!("step {n} reads the next file. ").repeat(20);
        let code = format!("fn step_{n}() -> u64 {{ {n} }}\n").repeat(100);
        for line in [
            json!({"type":"assistant","message":{"role":"assistant","content":[
                {"type":"text","text":prose},
                {"type":"tool_use","id":id,"name":"Read","input":{"file_path":format!("src/file_{n}.rs")}}]},
                "session_id":"%SID%"}),
            json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":id,"content":code,"is_error":false}]},
                "session_id":"%SID%"}),
        ] {
            out += &(line.to_string() + "\n");
        }
    }
    out + &json!({"type":"result","subtype":"success","is_error":false,"result":"done",
        "session_id":"%SID%","num_turns":1,"total_cost_usd":0.01,
        "usage":{"input_tokens":11,"output_tokens":4}})
    .to_string()
        + "\n"
}

/// Asks for a Bash call, then keeps working without an answer, so the ask falls out of the tail.
/// An ask claude settled itself goes first, as one answered on claude.ai would.
fn asking_turn() -> String {
    let ask = [
        json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"toolu_gone","name":"Bash","input":{"command":"ls"}}]},
            "session_id":"%SID%"}),
        json!({"type":"control_request","request_id":"req-gone","request":{"subtype":"can_use_tool",
            "tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"toolu_gone"}}),
        json!({"type":"control_cancel_request","request_id":"req-gone"}),
        json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"toolu_gone","content":"README.md","is_error":false}]},
            "session_id":"%SID%"}),
        json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"toolu_ask","name":"Bash","input":{"command":"make deploy"}}]},
            "session_id":"%SID%"}),
        json!({"type":"control_request","request_id":"req-ask","request":{"subtype":"can_use_tool",
            "tool_name":"Bash","input":{"command":"make deploy"},"tool_use_id":"toolu_ask"}}),
    ];
    let mut out: String = ask.iter().map(|line| line.to_string() + "\n").collect();
    let body = long_turn();
    // The long turn without its result, so the turn stays open behind the ask.
    out += &body[..body.trim_end().rfind('\n').map_or(0, |at| at + 1)];
    out
}

/// A client whose server-to-client bytes are counted, as a phone's link would carry them.
async fn tapped(socket: &Path) -> (Arc<RpcClient>, Arc<AtomicU64>) {
    let unix = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (mut server_rd, mut server_wr) = unix.into_split();
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (mut proxy_rd, mut proxy_wr) = tokio::io::split(proxy_end);
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut proxy_rd, &mut server_wr).await;
    });
    let down = Arc::new(AtomicU64::new(0));
    let counted = down.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match server_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            counted.fetch_add(n as u64, Relaxed);
            if proxy_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    });
    let client = RpcClient::connect(client_end).await.expect("handshake");
    (client, down)
}

/// One page and the bytes the phone received for it.
async fn read(
    phone: &RpcClient,
    down: &AtomicU64,
    sid: &str,
    cursor: LogCursor,
    limit: u32,
) -> (Result<SessionTurns, WireError>, u64) {
    let before = down.load(Relaxed);
    let page = phone
        .list_session_turns(sid.to_string(), cursor, limit)
        .await;
    (page, down.load(Relaxed) - before)
}

/// What a read to the newest row cost: round trips, bytes, and the time on the phone link.
fn cost(round_trips: usize, bytes: u64) -> serde_json::Value {
    let seconds = round_trips as f64 * LINK_ROUND_TRIP_S + bytes as f64 / LINK_BYTES_PER_S;
    json!({ "roundTrips": round_trips, "bytes": bytes, "secondsOnPhoneLink": (seconds * 10.0).round() / 10.0 })
}

async fn until_turn_done(events: &mut SessionEvents) {
    let done = tokio::time::timeout(IO, async {
        while let Some(frame) = events.recv().await {
            let event = match frame {
                SessionFrame::Item(item) => item.event,
                SessionFrame::Bare(event) => event,
            };
            if matches!(event, SessionEvent::TurnDone { .. }) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(done, Ok(true), "the turn never finished");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_session_open() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    std::fs::write(sandbox.root.join("turn.jsonl"), long_turn()).expect("write the turn");
    std::fs::write(sandbox.root.join("ask.jsonl"), asking_turn()).expect("write the asking turn");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("long_session_open", &sandbox);
    let server = Server::start(&sandbox).await;
    let desktop = server.client();
    let project = onboard_and_import(&mut j, &desktop, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &desktop, &project, "Long").await;
    let ws = text(&view, "/item/id");

    j.step("the desktop runs ten turns of 120 file reads");
    let (response, _, mut turns) = desktop
        .open_session_stream(StreamRequest::StartStageSession {
            workstream_id: ws.clone(),
            stage: Stage::General,
            input: "read".into(),
            attached_docs: vec![],
            opts: StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
            echo_user_messages: true,
            with_meta: false,
            all_partials: false,
        })
        .await
        .expect("start_stage_session");
    let StreamResponse::StartStageSession { session, .. } = response else {
        panic!("start_stage_session answered with another response");
    };
    let sid = session.id.clone();
    j.alias(&sid, "$SESSION");
    desktop
        .launch_and_converse(sid.clone())
        .await
        .expect("launch_and_converse");
    until_turn_done(&mut turns).await;
    for _ in 1..TURNS {
        desktop
            .send_user_message(sid.clone(), "again".into(), vec![], None, None, None)
            .await
            .expect("send_user_message");
        until_turn_done(&mut turns).await;
    }
    let log = desktop
        .get_session_log(sid.clone())
        .await
        .expect("get_session_log");
    let log_seqs: Vec<i64> = log.iter().map(|e| e.seq).collect();
    let newest = log.last().expect("the turns were logged");
    let newest_cursor = LogCursor::After {
        seq: newest.seq,
        created_at: newest.created_at,
    };
    j.note("log", json!({ "rows": log.len() }));

    let (phone, down) = tapped(server.socket()).await;
    let _phone_events = phone.take_events();

    j.step("the phone opens the session at its newest rows");
    let (mut cursor, mut forward_trips, mut forward_bytes) = (LogCursor::Empty, 0, 0);
    loop {
        let (page, bytes) = read(&phone, &down, &sid, cursor, PAGE_ROWS).await;
        let page = page.expect("a forward page");
        (forward_trips, forward_bytes) = (forward_trips + 1, forward_bytes + bytes);
        cursor = page.next_cursor;
        if page.remaining == 0 || forward_trips > 200 {
            break;
        }
    }
    let (tail, tail_bytes) = read(&phone, &down, &sid, LogCursor::Tail, TAIL_ROWS).await;
    let tail = tail.expect("the tail read");
    let holds_newest =
        tail.entries.last().map(|e| e.seq) == Some(newest.seq) && tail.remaining == 0;
    j.guard(
        "one read holds the newest row",
        holds_newest,
        json!({ "holdsNewest": holds_newest }),
    );
    j.guard(
        "that read fits one page's budget",
        tail_bytes <= PAGE_BOUND,
        json!({ "fits": tail_bytes <= PAGE_BOUND }),
    );
    let (after, _) = read(&phone, &down, &sid, tail.next_cursor, PAGE_ROWS).await;
    let follows = tail.next_cursor == newest_cursor
        && after.is_ok_and(|page| page.entries.is_empty() && page.remaining == 0);
    j.guard(
        "its cursor is the newest row, so reading on from it repeats nothing",
        follows,
        json!({ "follows": follows }),
    );
    let first = tail.entries.first().expect("the tail holds rows");
    let marks_older = tail.prev_cursor
        == Some(LogCursor::Before {
            seq: first.seq,
            created_at: first.created_at,
        });
    j.guard(
        "it names the read that continues back from its first row",
        marks_older,
        json!({ "marksOlder": marks_older }),
    );
    j.note(
        "firstOpen",
        json!({
            "forwardFromStart": cost(forward_trips, forward_bytes),
            "tailRead": cost(1, tail_bytes),
            "tailRows": tail.entries.len(),
        }),
    );

    j.step("paging back from the tail adds up to the whole log");
    let mut back: Vec<Vec<i64>> = vec![tail.entries.iter().map(|e| e.seq).collect()];
    let (mut from, mut pages, mut fits) = (tail.prev_cursor, 0, true);
    while let Some(cursor) = from {
        let (page, bytes) = read(&phone, &down, &sid, cursor, PAGE_ROWS).await;
        let page = page.expect("a page back");
        fits &= bytes <= PAGE_BOUND || page.entries.len() == 1;
        back.push(page.entries.iter().map(|e| e.seq).collect());
        from = page.prev_cursor;
        pages += 1;
        if pages > 200 {
            break;
        }
    }
    let joined: Vec<i64> = back.into_iter().rev().flatten().collect();
    j.guard(
        "the pages join without a gap or an overlap, ending at the first row",
        joined == log_seqs,
        json!({ "matches": joined == log_seqs }),
    );
    j.guard(
        "each page fits the budget or is a single row",
        fits,
        json!({ "fits": fits }),
    );
    j.note("backfill", json!({ "pages": pages }));

    j.step("a rewind refuses a cursor that reads back across it");
    let held = tail.prev_cursor.expect("the tail left older rows");
    let (_, _, _, rewind_events) = desktop
        .rewind_session_stream(sid.clone(), TURNS as u32 - 1, None)
        .await
        .expect("rewind_session");
    let deadline = tokio::time::Instant::now() + IO;
    let rewound = loop {
        let page = phone
            .list_session_turns(sid.clone(), LogCursor::Tail, TAIL_ROWS)
            .await
            .expect("a tail read after the rewind");
        if page.reset_rev > tail.reset_rev || tokio::time::Instant::now() > deadline {
            break page;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let moved = rewound.reset_rev > tail.reset_rev;
    drop(rewind_events);
    let refused = phone
        .list_session_turns(sid.clone(), held, PAGE_ROWS)
        .await
        .is_err_and(|e| e.code == "session_epoch_advanced");
    j.guard(
        "reading back from a row the rewind cut is refused as a moved log",
        refused,
        json!({ "refused": refused }),
    );
    // Read after the log, so rows the relaunch appends in between can only lengthen the tail.
    let log = desktop
        .get_session_log(sid.clone())
        .await
        .expect("get_session_log");
    let (after, _) = read(&phone, &down, &sid, LogCursor::Tail, TAIL_ROWS).await;
    let ends_new = after.is_ok_and(|page| {
        page.remaining == 0 && page.entries.last().map(|e| e.seq) >= log.last().map(|e| e.seq)
    });
    j.guard(
        "a tail read after it carries the moved resetRev and the new newest row",
        moved && ends_new,
        json!({ "moved": moved, "endsAtNewest": ends_new }),
    );

    j.step("a tail read carries the ask the harness still waits on");
    let (response, _, mut asking) = desktop
        .open_session_stream(StreamRequest::StartStageSession {
            workstream_id: ws.clone(),
            stage: Stage::General,
            input: "ASK_FIRST".into(),
            attached_docs: vec![],
            opts: StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
            echo_user_messages: true,
            with_meta: false,
            all_partials: false,
        })
        .await
        .expect("start_stage_session");
    let StreamResponse::StartStageSession { session, .. } = response else {
        panic!("start_stage_session answered with another response");
    };
    let asker = session.id.clone();
    j.alias(&asker, "$ASKER");
    desktop
        .launch_and_converse(asker.clone())
        .await
        .expect("launch_and_converse");
    let deadline = tokio::time::Instant::now() + IO;
    while desktop
        .get_session_log(asker.clone())
        .await
        .map_or(0, |log| log.len())
        < 2 * STEPS
        && tokio::time::Instant::now() < deadline
    {
        let _ = tokio::time::timeout(Duration::from_millis(100), asking.recv()).await;
    }
    let (page, bytes) = read(&phone, &down, &asker, LogCursor::Tail, TAIL_ROWS).await;
    let page = page.expect("a tail read of the asking session");
    let older = page.entries.first().map_or(i64::MAX, |e| e.seq);
    let carried = matches!(page.open_asks.as_slice(),
        [ask] if ask.seq < older && matches!(&ask.event, SessionEvent::PermissionAsk { request_id, .. } if request_id == "req-ask"));
    j.guard(
        "it carries the one ask older than the page, within the page's budget",
        carried && bytes <= PAGE_BOUND,
        json!({ "carried": carried, "fits": bytes <= PAGE_BOUND }),
    );
    desktop
        .respond_permission(asker.clone(), "req-ask".into(), true, None)
        .await
        .expect("respond_permission");
    let (page, _) = read(&phone, &down, &asker, LogCursor::Tail, TAIL_ROWS).await;
    let answered = page.is_ok_and(|page| page.open_asks.is_empty());
    j.guard(
        "an answered ask is no longer carried",
        answered,
        json!({ "answered": answered }),
    );

    j.step("the answer is logged, and a tail read names every ask still waiting");
    desktop
        .announce_client(
            "desktop".into(),
            "e2e".into(),
            TARGETED_EVENTS.map(str::to_string).to_vec(),
        )
        .await
        .expect("announce_client");
    let logged = answer_rows(&desktop, &asker).await;
    let tail_rows: Vec<serde_json::Value> = desktop
        .list_session_turns(asker.clone(), LogCursor::Tail, TAIL_ROWS)
        .await
        .expect("a tail read of the asking session")
        .entries
        .into_iter()
        .filter(|e| e.kind == "permissionResolved")
        .filter_map(|e| serde_json::to_value(e.event).ok())
        .collect();
    let (page, _) = read(&phone, &down, &asker, LogCursor::Tail, TAIL_ROWS).await;
    let waiting = page
        .ok()
        .and_then(|page| serde_json::to_value(page).ok())
        .map(|page| page["openRequestIds"].clone());
    j.guard(
        "the log closes the ask by its id, and the tail read says nothing still waits",
        logged == [json!({ "kind": "permissionResolved", "requestId": "req-ask", "allow": true })]
            && tail_rows == logged
            && waiting == Some(json!([])),
        json!({ "logged": logged, "inTailRead": tail_rows == logged, "openRequestIds": waiting }),
    );
    let withheld = phone.get_session_log(asker.clone()).await.ok().map(|log| {
        log.iter()
            .filter(|e| e.kind == "permissionResolved")
            .count()
    });
    j.guard(
        "a client that never said it reads answer rows gets none",
        withheld == Some(0),
        json!({ "answerRows": withheld }),
    );

    j.finish();
}

/// The asker's answer rows, as JSON so this reads on a build without them; waits for the first.
async fn answer_rows(client: &RpcClient, sid: &str) -> Vec<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rows: Vec<serde_json::Value> = client
            .get_session_log(sid.to_string())
            .await
            .expect("get_session_log")
            .into_iter()
            .filter(|e| e.kind == "permissionResolved")
            .filter_map(|e| serde_json::to_value(e.event).ok())
            .collect();
        if !rows.is_empty() || tokio::time::Instant::now() >= deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
