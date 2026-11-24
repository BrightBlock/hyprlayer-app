//! What a session costs a client on the wire: catch-up pages are capped in bytes, a
//! watcher is not sent typing it would discard, and typing deltas share frames.
#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyprlayer_core::models::SessionStreamItem;
use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Sandbox, Server, Value};
use hyprlayer_proto::frame::{decode_stream_frame, Decoder, Frame};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const IO: Duration = Duration::from_secs(60);
/// The server's page budget, plus room for one page's row envelopes.
const PAGE_BOUND: usize = 256 * 1024 + 16 * 1024;
/// The server's coalescing tick.
const TICK: Duration = Duration::from_millis(50);
const DELTAS: usize = 200;
const SUBAGENT_DELTAS: usize = 20;
const STEPS: usize = 48;
const OVERSIZED: usize = 600 * 1024;

/// Emits `<sandbox>/deltas` one word at a time, then replays `<sandbox>/turn.jsonl`.
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
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  if [ -f "$cfg/deltas" ]; then
    while IFS=' ' read -r parent word; do
      printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"%s "}},"parent_tool_use_id":%s}\n' "$word" "$parent"
      sleep 0.005
    done < "$cfg/deltas"
  fi
  sed -e "s/%SID%/$sid/g" "$cfg/turn.jsonl"
done
"#;

/// One client connection's server-to-client frames, as a proxy in front of it saw them.
#[derive(Default)]
struct Wire {
    frames: Mutex<Vec<(Instant, Frame)>>,
}

impl Wire {
    fn take(&self) -> Vec<(Instant, Frame)> {
        std::mem::take(&mut self.frames.lock().unwrap())
    }
}

async fn tapped(socket: &Path) -> (Arc<RpcClient>, Arc<Wire>) {
    let unix = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (mut server_rd, mut server_wr) = unix.into_split();
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (mut proxy_rd, mut proxy_wr) = tokio::io::split(proxy_end);
    let wire = Arc::new(Wire::default());
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut proxy_rd, &mut server_wr).await;
    });
    let down = wire.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        let mut decoder = Decoder::new();
        loop {
            let n = match server_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            decoder.feed(&buf[..n]);
            let at = Instant::now();
            while let Ok(Some(frame)) = decoder.next_frame() {
                down.frames.lock().unwrap().push((at, frame));
            }
            if proxy_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    });
    let client = RpcClient::connect(client_end).await.expect("handshake");
    (client, wire)
}

async fn announce(client: &RpcClient, id: &str) {
    client
        .announce_client(
            id.into(),
            format!("{id}-device"),
            TARGETED_EVENTS.map(str::to_string).to_vec(),
        )
        .await
        .expect("announce_client");
}

/// The data frames of one stream, decoded as raw JSON with their arrival time.
fn stream_frames(frames: &[(Instant, Frame)], stream_id: u64) -> Vec<(Instant, usize, Value)> {
    frames
        .iter()
        .filter_map(|(at, frame)| {
            let Frame::Data(payload) = frame else {
                return None;
            };
            let (id, body) = decode_stream_frame(payload)?;
            let value = serde_json::from_slice(body).ok()?;
            (id == stream_id).then_some((*at, payload.len() + 5, value))
        })
        .collect()
}

fn largest_control(frames: &[(Instant, Frame)]) -> usize {
    frames
        .iter()
        .filter_map(|(_, frame)| match frame {
            Frame::Control(line) => Some(line.len() + 2),
            Frame::Data(_) => None,
        })
        .max()
        .unwrap_or(0)
}

fn kind(item: &Value) -> &str {
    item["event"]["kind"].as_str().unwrap_or_default()
}

fn author_id(item: &Value) -> Option<&str> {
    item["author"]["clientId"].as_str()
}

fn has_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().any(has_null),
        Value::Object(map) => map.values().any(has_null),
        _ => false,
    }
}

fn partials(items: &[(Instant, usize, Value)], main_thread: bool) -> Vec<&Value> {
    items
        .iter()
        .map(|(_, _, item)| item)
        .filter(|item| kind(item) == "partialText")
        .filter(|item| item["event"]["parentId"].as_str().is_none() == main_thread)
        .collect()
}

fn joined(items: &[&Value]) -> String {
    items
        .iter()
        .filter_map(|item| item["event"]["text"].as_str())
        .collect()
}

/// Reads until an item matches `stop`.
async fn until(events: &mut SessionEvents, stop: impl Fn(&SessionStreamItem) -> bool) {
    let done = tokio::time::timeout(IO, async {
        while let Some(frame) = events.recv().await {
            if matches!(frame, SessionFrame::Item(item) if stop(&item)) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(done, Ok(true), "the stream ended or stalled");
}

/// Reads until a turn end committed after `after_seq`.
async fn until_turn_done(events: &mut SessionEvents, after_seq: i64) {
    until(events, |item| {
        matches!(item.event, SessionEvent::TurnDone { .. })
            && item.seq.is_some_and(|seq| seq > after_seq)
    })
    .await;
}

fn line(value: Value) -> String {
    value.to_string() + "\n"
}

fn result_line() -> String {
    line(
        json!({"type":"result","subtype":"success","is_error":false,"result":"done",
        "session_id":"%SID%","num_turns":1,"total_cost_usd":0.01,
        "usage":{"input_tokens":1,"output_tokens":1}}),
    )
}

/// `STEPS` reads of a 20 KB file and one of a file larger than a page.
fn long_turn() -> String {
    let body = "fn main() { println!(\"hello\"); }\n".repeat(20 * 1024 / 33);
    let mut out = String::new();
    for n in 0..=STEPS {
        let id = format!("toolu_{n}");
        let content = if n == STEPS {
            "x".repeat(OVERSIZED)
        } else {
            body.clone()
        };
        out += &line(
            json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":id,"name":"Read","input":{"file_path":format!("src/file_{n}.rs")}}]},
            "session_id":"%SID%"}),
        );
        out += &line(json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":id,"content":content,"is_error":false}]},
            "session_id":"%SID%"}));
    }
    out += &line(
        json!({"type":"assistant","message":{"role":"assistant","content":[
        {"type":"text","text":"read them all"}]},"session_id":"%SID%"}),
    );
    out + &result_line()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_stream_payload() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("session_stream_payload", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Payload").await;
    let ws = text(&view, "/item/id");

    let (desktop, desktop_wire) = tapped(server.socket()).await;
    let (phone, phone_wire) = tapped(server.socket()).await;
    let _desktop_events = desktop.take_events();
    let _phone_events = phone.take_events();
    announce(&desktop, "desktop").await;
    announce(&phone, "phone").await;

    j.step("the desktop runs a turn that reads many large files");
    std::fs::write(sandbox.root.join("turn.jsonl"), long_turn()).expect("write the turn");
    let (response, desktop_stream_id, mut desktop_stream) = desktop
        .open_session_stream(StreamRequest::StartStageSession {
            workstream_id: ws,
            stage: Stage::General,
            input: "read".into(),
            attached_docs: vec![],
            opts: StreamStartOpts {
                harness: Some(Harness::Claude),
                client_message_id: Some("cmid-read".into()),
                ..Default::default()
            },
            echo_user_messages: true,
            with_meta: true,
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
    until_turn_done(&mut desktop_stream, -1).await;
    let log = client
        .get_session_log(sid.clone())
        .await
        .expect("get_session_log");
    let log_seqs: Vec<i64> = log.iter().map(|e| e.seq).collect();
    let last_seq = *log_seqs.last().expect("the turn was logged");

    j.step("the phone catches up in pages capped by size");
    let mut cursor = LogCursor::Empty;
    let mut paged_seqs = Vec::new();
    let mut pages: Vec<(usize, usize, bool)> = Vec::new();
    loop {
        phone_wire.take();
        let page = phone
            .list_session_turns(sid.clone(), cursor, 2000)
            .await
            .expect("list_session_turns");
        let frame = largest_control(&phone_wire.take());
        let oversized = page.entries.iter().any(|e| match &e.event {
            SessionEvent::ToolResult { content, .. } => content.len() >= OVERSIZED,
            _ => false,
        });
        pages.push((frame, page.entries.len(), oversized));
        paged_seqs.extend(page.entries.iter().map(|e| e.seq));
        cursor = page.next_cursor;
        if page.remaining == 0 || pages.len() > 200 {
            break;
        }
    }
    let fits = pages
        .iter()
        .all(|&(frame, rows, _)| frame <= PAGE_BOUND || rows == 1);
    j.guard(
        "each page fits 256 KiB or is a single row",
        fits,
        json!({ "fits": fits }),
    );
    let alone = pages
        .iter()
        .any(|&(_, rows, oversized)| oversized && rows == 1);
    j.guard(
        "a row larger than a page comes back alone, and the read moves past it",
        alone && paged_seqs.last() == Some(&last_seq),
        json!({ "alone": alone, "reachedEnd": paged_seqs.last() == Some(&last_seq) }),
    );
    j.guard(
        "the pages add up to the whole log",
        paged_seqs == log_seqs,
        json!({ "matches": paged_seqs == log_seqs }),
    );
    j.note(
        "pages",
        json!({
            "count": pages.len(),
            "rows": paged_seqs.len(),
            "largestFrame": pages.iter().map(|p| p.0).max(),
            "totalBytes": pages.iter().map(|p| p.0).sum::<usize>(),
        }),
    );

    j.step("the desktop streams a reply while the phone watches");
    let words: Vec<String> = (0..DELTAS).map(|n| format!("word{n:03}")).collect();
    let full: String = words.iter().map(|w| format!("{w} ")).collect();
    let mut deltas: String = words.iter().map(|w| format!("null {w}\n")).collect();
    deltas += &(0..SUBAGENT_DELTAS)
        .map(|n| format!("\"toolu_sub\" sub{n:02}\n"))
        .collect::<String>();
    std::fs::write(sandbox.root.join("deltas"), deltas).expect("write the deltas");
    std::fs::write(
        sandbox.root.join("turn.jsonl"),
        line(
            json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":full}]},"session_id":"%SID%"}),
        ) + &result_line(),
    )
    .expect("write the turn");
    let attach = |all_partials| StreamRequest::AttachSession {
        session_id: sid.clone(),
        echo_user_messages: true,
        with_meta: true,
        all_partials,
    };
    let (_, phone_stream_id, mut phone_stream) = phone
        .open_session_stream(attach(false))
        .await
        .expect("the phone attaches");
    let (_, every_stream_id, mut every_stream) = phone
        .open_session_stream(attach(true))
        .await
        .expect("the phone attaches asking for every partial");
    let replayed = |item: &SessionStreamItem| item.seq == Some(last_seq);
    until(&mut phone_stream, replayed).await;
    until(&mut every_stream, replayed).await;
    desktop_wire.take();
    phone_wire.take();
    let sent_at = Instant::now();
    desktop
        .send_user_message(
            sid.clone(),
            "stream".into(),
            vec![],
            None,
            Some("cmid-stream".into()),
            None,
        )
        .await
        .expect("send_user_message");
    until_turn_done(&mut desktop_stream, last_seq).await;
    until_turn_done(&mut phone_stream, last_seq).await;
    until_turn_done(&mut every_stream, last_seq).await;

    let phone_frames = phone_wire.take();
    let watched = stream_frames(&phone_frames, phone_stream_id);
    let foreign = partials(&watched, true)
        .into_iter()
        .chain(partials(&watched, false))
        .filter(|item| author_id(item) == Some("desktop"))
        .count();
    j.guard(
        "the phone is sent none of the desktop's typing",
        foreign == 0,
        json!({ "foreignPartials": foreign }),
    );
    let reply = watched.iter().any(|(_, _, item)| {
        kind(item) == "assistantText"
            && item["event"]["text"] == full.as_str()
            && author_id(item) == Some("desktop")
    });
    let ended = watched
        .iter()
        .any(|(_, _, item)| kind(item) == "turnDone" && item["seq"].as_i64() > Some(last_seq));
    j.guard(
        "the phone still gets the desktop's reply and the turn's end",
        reply && ended,
        json!({ "reply": reply, "turnDone": ended }),
    );

    let log = client
        .get_session_log(sid.clone())
        .await
        .expect("get_session_log");
    let live_rows: Vec<&Value> = watched
        .iter()
        .map(|(_, _, item)| item)
        .filter(|item| item["seq"].as_i64() > Some(last_seq))
        .collect();
    let stamped = !live_rows.is_empty()
        && live_rows.iter().all(|item| {
            let row = log.iter().find(|e| Some(e.seq) == item["seq"].as_i64());
            row.map(|e| e.created_at) == item["createdAt"].as_f64()
        });
    let resumes = match live_rows.last() {
        Some(item) => {
            let cursor = LogCursor::After {
                seq: item["seq"].as_i64().unwrap_or_default(),
                created_at: item["createdAt"].as_f64().unwrap_or_default(),
            };
            phone
                .list_session_turns(sid.clone(), cursor, 2000)
                .await
                .is_ok_and(|page| page.entries.is_empty() && page.remaining == 0)
        }
        None => false,
    };
    j.guard(
        "each live row carries the createdAt that makes it a resume cursor",
        stamped && resumes,
        json!({ "stamped": stamped, "resumes": resumes }),
    );

    let own = stream_frames(&desktop_wire.take(), desktop_stream_id);
    let typed = partials(&own, true);
    j.guard(
        "the desktop's own deltas add up to its reply",
        joined(&typed) == full,
        json!({ "matches": joined(&typed) == full }),
    );
    let subagent = partials(&own, false).len();
    j.guard(
        "the desktop is sent no subagent typing, which it never renders",
        subagent == 0,
        json!({ "subagentPartials": subagent }),
    );
    let partial_frames: Vec<&(Instant, usize, Value)> = own
        .iter()
        .filter(|(_, _, item)| kind(item) == "partialText")
        .collect();
    // From the send, so a stalled proxy can only lengthen the span the bound allows.
    let elapsed = partial_frames
        .last()
        .map_or(Duration::ZERO, |last| last.0.duration_since(sent_at));
    let allowed = (elapsed.as_secs_f64() / TICK.as_secs_f64()) as usize + 3;
    j.guard(
        "deltas share frames: at most one per 50 ms tick",
        partial_frames.len() <= allowed,
        json!({ "withinTick": partial_frames.len() <= allowed }),
    );
    let null_free = !partial_frames.iter().any(|(_, _, item)| has_null(item));
    j.guard(
        "a partial frame carries no null field",
        null_free,
        json!({ "nullFree": null_free }),
    );
    j.step("a stream that asks for every partial still gets them");
    let every = stream_frames(&phone_frames, every_stream_id);
    let subagent_text: String = (0..SUBAGENT_DELTAS)
        .map(|n| format!("sub{n:02} "))
        .collect();
    let main_typed = joined(&partials(&every, true)) == full;
    let sub_typed = joined(&partials(&every, false)) == subagent_text;
    j.guard(
        "the desktop's typing and its subagent's reach the opted-in stream",
        main_typed && sub_typed,
        json!({ "main": main_typed, "subagent": sub_typed }),
    );

    j.step("a turn no device sent still types live on the phone");
    let turn_seq = log.last().map_or(last_seq, |e| e.seq);
    phone_wire.take();
    client
        .send_user_message(sid, "again".into(), vec![], None, None, None)
        .await
        .expect("send_user_message");
    until_turn_done(&mut phone_stream, turn_seq).await;
    let unattributed = stream_frames(&phone_wire.take(), phone_stream_id);
    let typed_live = joined(&partials(&unattributed, true)) == full;
    j.guard(
        "the phone gets an unattributed turn's typing, as it would an agent's",
        typed_live,
        json!({ "typed": typed_live }),
    );

    j.note(
        "typing",
        json!({
            "deltas": DELTAS + SUBAGENT_DELTAS,
            "textBytes": full.len(),
            "partialFrames": partial_frames.len(),
            "partialWireBytes": partial_frames.iter().map(|f| f.1).sum::<usize>(),
            "elapsedMs": elapsed.as_millis(),
            "phoneStreamBytes": watched.iter().map(|f| f.1).sum::<usize>(),
        }),
    );

    j.finish();
}
