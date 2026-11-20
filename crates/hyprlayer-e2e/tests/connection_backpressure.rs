//! A phone-like client on one connection: a reader that stalls, then crawls at
//! 256 KiB/s. Memory stays bounded on both ends, small calls overtake bulk
//! data and large responses, and missed runtime events are reported, not lost.
#![cfg(unix)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, RuntimeEvent, Sandbox, Server, Value};
use hyprlayer_proto::frame::{Decoder, Frame};
use hyprlayer_server::client::{GapPacer, RpcClient};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::rpc::WatchedSession;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const IO: Duration = Duration::from_secs(60);
const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;
const PHONE_BPS: u64 = 256 * KIB;
const FLOOD: &str = "yes 0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";
/// Steps per turn, each a few `SessionRevised` events.
const STEPS: usize = 120;
/// More events than the server queues for a stalled reader.
const STALL_EVENTS: u64 = 400;
/// About 8 s on the phone's link as base64.
const LARGE_RESPONSE_BYTES: usize = 1536 * 1024;

const SERVER_GROWTH_LIMIT: u64 = 48 * MIB;
const CLIENT_GROWTH_LIMIT: u64 = 64 * MIB;
const SMALL_CALL_LIMIT: Duration = Duration::from_secs(3);
/// Slower than a turn's events, so the flood keeps the event queue overflowing.
const SLOW_BPS: u64 = 32 * KIB;
const FLOOD_FOR: Duration = Duration::from_secs(12);
/// The client's `GAP_INTERVAL`, less scheduling slack.
const REREAD_SPACING: Duration = Duration::from_millis(4500);

/// Replays `<sandbox>/turn.jsonl` for every user line it is handed.
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
  sed -e "s/%SID%/$sid/g" -e "s/%TURN%/$turn/g" "$cfg/turn.jsonl"
done
"#;

/// The server-to-client half of one connection, as a proxy that can stall or throttle it sees it.
#[derive(Default)]
struct Link {
    stalled: AtomicBool,
    bps: AtomicU64,
    tap: Mutex<Tap>,
}

#[derive(Default)]
struct Tap {
    decoder: Decoder,
    /// `sessionRevised` events per session id.
    revised: std::collections::HashMap<String, u64>,
    events: u64,
    reported_missed: u64,
    markers: u64,
}

impl Tap {
    fn note(&mut self, frame: &Frame) {
        let Frame::Control(line) = frame else { return };
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if let Some(event) = value.pointer("/event/event") {
            self.events += 1;
            if let Some(session) = event
                .pointer("/sessionRevised/sessionId")
                .and_then(Value::as_str)
            {
                *self.revised.entry(session.to_string()).or_default() += 1;
            }
        }
        if let Some(count) = value
            .pointer("/events_missed/count")
            .and_then(Value::as_u64)
        {
            self.reported_missed += count;
            self.markers += 1;
        }
    }
}

impl Link {
    fn revised(&self, session: &str) -> u64 {
        self.tap
            .lock()
            .unwrap()
            .revised
            .get(session)
            .copied()
            .unwrap_or(0)
    }
    fn reported_missed(&self) -> u64 {
        self.tap.lock().unwrap().reported_missed
    }
    fn markers(&self) -> u64 {
        self.tap.lock().unwrap().markers
    }
}

/// What the window re-reads after a gap: the session lists, the transcript watch, settings.
async fn reread(client: &RpcClient, session: &str) {
    let _ = client.list_active_sessions().await;
    watch(client, session).await;
    let _ = client.get_app_settings().await;
}

async fn phone(socket: &Path) -> (Arc<RpcClient>, Arc<Link>) {
    let unix = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (mut srv_rd, mut srv_wr) = unix.into_split();
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (mut cli_rd, mut cli_wr) = tokio::io::split(proxy_end);
    let link = Arc::new(Link::default());
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match cli_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if srv_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    });
    let down = link.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            while down.stalled.load(Relaxed) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let n = match srv_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            {
                let mut tap = down.tap.lock().unwrap();
                tap.decoder.feed(&buf[..n]);
                while let Ok(Some(frame)) = tap.decoder.next_frame() {
                    tap.note(&frame);
                }
            }
            if cli_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
            let bps = down.bps.load(Relaxed);
            if bps > 0 {
                tokio::time::sleep(Duration::from_secs_f64(n as f64 / bps as f64)).await;
            }
        }
    });
    let client = RpcClient::connect(client_end).await.expect("handshake");
    (client, link)
}

fn rss_bytes(pid: u32) -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        * KIB
}

/// The highest RSS seen while sampling for `over`.
async fn peak_rss(pid: u32, over: Duration) -> u64 {
    let deadline = Instant::now() + over;
    let mut peak = rss_bytes(pid);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
        peak = peak.max(rss_bytes(pid));
    }
    peak
}

fn mib(n: u64) -> f64 {
    n as f64 / MIB as f64
}

fn write_turn(sandbox: &Sandbox) {
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
            out += &line.to_string();
            out.push('\n');
        }
    }
    out += &json!({"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%SID%",
        "num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4},
        "modelUsage":{"claude-e2e":{"contextWindow":200000}}})
    .to_string();
    out.push('\n');
    std::fs::write(sandbox.root.join("turn.jsonl"), out).expect("write the turn script");
}

async fn announce(client: &RpcClient, id: &str) {
    client
        .announce_client(
            id.into(),
            format!("device-{id}"),
            TARGETED_EVENTS.map(str::to_string).to_vec(),
        )
        .await
        .expect("announce_client");
}

async fn watch(client: &RpcClient, session: &str) {
    client
        .watch_sessions(vec![WatchedSession {
            session_id: session.to_string(),
            rev: 0,
        }])
        .await
        .expect("watch_sessions");
}

async fn start_session(client: &RpcClient, ws: &str) -> (String, SessionEvents) {
    let (response, _stream, events) = client
        .open_session_stream(StreamRequest::StartStageSession {
            workstream_id: ws.to_string(),
            stage: Stage::General,
            input: "go".into(),
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
    client
        .launch_and_converse(session.id.clone())
        .await
        .expect("launch_and_converse");
    (session.id, events)
}

async fn until_turn_done(events: &mut SessionEvents) {
    loop {
        match tokio::time::timeout(IO, events.recv()).await {
            Ok(Some(frame)) => {
                let event = match frame {
                    SessionFrame::Item(item) => item.event,
                    SessionFrame::Bare(event) => event,
                };
                if matches!(event, SessionEvent::TurnDone { .. }) {
                    return;
                }
            }
            other => panic!("the turn never finished: {:?}", other.is_ok()),
        }
    }
}

async fn send_turn(client: &RpcClient, session: &str, events: &mut SessionEvents) {
    client
        .send_user_message(
            session.to_string(),
            "again".into(),
            vec![],
            None,
            None,
            None,
        )
        .await
        .expect("send_user_message");
    until_turn_done(events).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_backpressure() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    write_turn(&sandbox);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("connection_backpressure", &sandbox);
    let server = Server::start(&sandbox).await;
    let desktop = server.client();
    let project = onboard_and_import(&mut j, &desktop, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &desktop, &project, "Backpressure").await;
    let ws = text(&view, "/item/id");
    let pid = server.pid();

    let (session, mut turns) = start_session(&desktop, &ws).await;
    until_turn_done(&mut turns).await;
    send_turn(&desktop, &session, &mut turns).await;
    announce(&desktop, "desktop").await;
    watch(&desktop, &session).await;
    let desktop_revised = Arc::new(AtomicU64::new(0));
    {
        let mut events = desktop.take_events().expect("the desktop's runtime events");
        let (counter, session) = (desktop_revised.clone(), session.clone());
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                if matches!(&event, RuntimeEvent::SessionRevised { session_id, .. } if *session_id == session)
                {
                    counter.fetch_add(1, Relaxed);
                }
            }
        });
    }

    let (phone, link) = phone(server.socket()).await;
    {
        let mut events = phone.take_events().expect("the phone's runtime events");
        tokio::spawn(async move { while events.recv().await.is_some() {} });
    }
    announce(&phone, "phone").await;
    watch(&phone, &session).await;

    j.step("a stalled reader with a printing terminal holds the server's memory");
    let (terminal, _stream, mut output) = phone
        .spawn_shell_terminal_stream(FLOOD.into(), 120, 32)
        .await
        .expect("spawn_shell_terminal");
    let printed = Arc::new(AtomicU64::new(0));
    {
        let printed = printed.clone();
        tokio::spawn(async move {
            while let Some(TerminalEvent::Output { data }) = output.recv().await {
                printed.fetch_add(data.len() as u64, Relaxed);
            }
        });
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    j.guard(
        "the terminal was printing before the stall",
        printed.load(Relaxed) > 0,
        json!({}),
    );
    link.stalled.store(true, Relaxed);
    let before = rss_bytes(pid);
    let turns_during_stall = {
        let (desktop, session, sent) = (desktop.clone(), session.clone(), desktop_revised.clone());
        let at_stall = sent.load(Relaxed);
        tokio::spawn(async move {
            for _ in 0..8 {
                if sent.load(Relaxed) - at_stall >= STALL_EVENTS {
                    break;
                }
                send_turn(&desktop, &session, &mut turns).await;
            }
            turns
        })
    };
    let peak = peak_rss(pid, Duration::from_secs(8)).await;
    let mut turns = turns_during_stall
        .await
        .expect("turns while the phone stalls");
    let growth = peak.saturating_sub(before);
    j.note(
        "server_rss_mib",
        json!({ "before": mib(before), "peak_while_stalled": mib(peak) }),
    );
    j.guard(
        "server memory stays bounded while one reader stalls",
        growth < SERVER_GROWTH_LIMIT,
        json!({ "limit_mib": mib(SERVER_GROWTH_LIMIT) }),
    );

    j.step("a small call issued during the stall overtakes the terminal's backlog");
    let pending = {
        let phone = phone.clone();
        tokio::spawn(async move { phone.default_shell_name().await.is_ok() })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    link.bps.store(PHONE_BPS, Relaxed);
    let resumed = Instant::now();
    link.stalled.store(false, Relaxed);
    let answered = tokio::time::timeout(Duration::from_secs(15), pending).await;
    let waited = resumed.elapsed();
    j.note(
        "small_call_after_stall_ms",
        json!(waited.as_millis() as u64),
    );
    j.guard(
        "the call is answered promptly once the link moves again",
        matches!(answered, Ok(Ok(true))) && waited < SMALL_CALL_LIMIT,
        json!({ "limit_ms": SMALL_CALL_LIMIT.as_millis() as u64 }),
    );

    j.step("events sent during the stall are delivered or reported missed");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let sent = desktop_revised.load(Relaxed);
    let delivered = link.revised(&session);
    let missed = link.reported_missed();
    j.note(
        "events_during_stall",
        json!({ "sent": sent, "delivered": delivered, "reported_missed": missed }),
    );
    j.guard(
        "no event is lost silently",
        sent > 0 && delivered + missed >= sent,
        json!({ "sent_some": sent > 0 }),
    );
    let surfaced = *phone.events_missed().borrow();
    j.guard(
        "the client hears the gap, so it can re-read what the events would have changed",
        missed > 0 && surfaced >= missed,
        json!({}),
    );
    let _ = desktop.terminal_close(terminal).await;

    j.step("a small call is not held behind a large response");
    let blob: Vec<u8> = (0..LARGE_RESPONSE_BYTES)
        .map(|i| (i * 31 % 251) as u8)
        .collect();
    let attachment = desktop
        .save_attachment(ws.clone(), "large.bin".into(), STANDARD.encode(&blob))
        .await
        .expect("save_attachment");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let large = {
        let phone = phone.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let read =
                tokio::time::timeout(Duration::from_secs(120), phone.read_attachment(attachment))
                    .await;
            (matches!(read, Ok(Ok(_))), started.elapsed())
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = Instant::now();
    let small = tokio::time::timeout(Duration::from_secs(30), phone.default_shell_name()).await;
    let small_took = started.elapsed();
    let (large_ok, large_took) = large.await.expect("the large read");
    j.note(
        "large_response_ms",
        json!({ "large": large_took.as_millis() as u64, "small_behind_it": small_took.as_millis() as u64 }),
    );
    j.guard(
        "the large response is slow on this link",
        large_ok && large_took > SMALL_CALL_LIMIT,
        json!({ "slower_than_ms": SMALL_CALL_LIMIT.as_millis() as u64 }),
    );
    j.guard(
        "the small call issued after it is answered first",
        matches!(small, Ok(Ok(_))) && small_took < SMALL_CALL_LIMIT,
        json!({ "limit_ms": SMALL_CALL_LIMIT.as_millis() as u64 }),
    );

    j.step("a flood of events on a slow link re-reads at a bounded rate");
    link.bps.store(SLOW_BPS, Relaxed);
    let markers_before = link.markers();
    let rereads = Arc::new(Mutex::new(Vec::new()));
    let rereading = {
        let (phone, rereads, session) = (phone.clone(), rereads.clone(), session.clone());
        let mut gaps = GapPacer::new(phone.events_missed());
        tokio::spawn(async move {
            while gaps.next().await {
                rereads.lock().unwrap().push(Instant::now());
                reread(&phone, &session).await;
            }
        })
    };
    let flood_ends = Instant::now() + FLOOD_FOR;
    let mut flood_turns = 0;
    while Instant::now() < flood_ends {
        send_turn(&desktop, &session, &mut turns).await;
        flood_turns += 1;
    }
    tokio::time::sleep(Duration::from_secs(8)).await;
    rereading.abort();
    let rereads = rereads.lock().unwrap().clone();
    let spacing: Vec<f64> = rereads
        .windows(2)
        .map(|w| (w[1] - w[0]).as_secs_f64())
        .collect();
    j.note(
        "flood",
        json!({
            "turns": flood_turns,
            "markers": link.markers() - markers_before,
            "rereads": rereads.len(),
            "spacing_s": spacing,
        }),
    );
    j.guard(
        "the flood is lossy",
        link.markers() > markers_before,
        json!({}),
    );
    j.guard(
        "re-reads stay apart however many gaps arrive",
        !rereads.is_empty() && spacing.iter().all(|s| *s >= REREAD_SPACING.as_secs_f64()),
        json!({ "at_least_s": REREAD_SPACING.as_secs_f64() }),
    );

    j.step("a stream nobody reads holds the client's memory");
    let reader = server.connect().await;
    let me = std::process::id();
    let before = rss_bytes(me);
    let (terminal, _stream, mut unread) = reader
        .spawn_shell_terminal_stream(FLOOD.into(), 120, 32)
        .await
        .expect("spawn_shell_terminal");
    let held = {
        let reader = reader.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let started = Instant::now();
            let answer =
                tokio::time::timeout(Duration::from_secs(15), reader.default_shell_name()).await;
            (matches!(answer, Ok(Ok(_))), started.elapsed())
        })
    };
    let peak = peak_rss(me, Duration::from_secs(6)).await;
    let growth = peak.saturating_sub(before);
    let (answered, call_took) = held.await.expect("the call during the hold");
    let mut waiting = 0u64;
    while let Ok(Some(TerminalEvent::Output { data })) =
        tokio::time::timeout(Duration::from_millis(300), unread.recv()).await
    {
        waiting += data.len() as u64;
        if waiting > MIB {
            break;
        }
    }
    j.note(
        "client_rss_mib",
        json!({ "before": mib(before), "peak_unread": mib(peak), "first_read_kib": waiting / KIB }),
    );
    j.guard(
        "the unread stream had output waiting",
        waiting > 0,
        json!({}),
    );
    j.guard(
        "client memory stays bounded while a stream goes unread",
        growth < CLIENT_GROWTH_LIMIT,
        json!({ "limit_mib": mib(CLIENT_GROWTH_LIMIT) }),
    );
    j.note("call_during_unread_ms", json!(call_took.as_millis() as u64));
    j.guard(
        "a call on the same connection still answers",
        answered && call_took < SMALL_CALL_LIMIT,
        json!({ "limit_ms": SMALL_CALL_LIMIT.as_millis() as u64 }),
    );
    let _ = reader.terminal_close(terminal).await;
    drop(unread);

    j.finish();
}
