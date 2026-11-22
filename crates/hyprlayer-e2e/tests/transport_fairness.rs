//! A phone on a slow, stalling or buffering link, in both directions: small
//! calls answer within `SMALL_CALL_LIMIT` whatever else the connection
//! carries, every stream keeps its share, and pipelined reads hold bounded
//! server memory.
#![cfg(unix)]

use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::link::{
    far_phone, far_phone_announcing, phone, uncapped, DEEP_LINK_BYTES, DEEP_UPLINK_BYTES,
};
use hyprlayer_e2e::{json, next_output, text, Journey, Sandbox, Server, RAW_DESKTOP};
use hyprlayer_proto::Features;
use hyprlayer_server::client::{RpcClient, TerminalStream};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::rpc::WatchedSession;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;
const PHONE_BPS: u64 = 256 * KIB;
const SLOW_BPS: u64 = 64 * KIB;
const FAST_BPS: u64 = 512 * KIB;
const IO: Duration = Duration::from_secs(120);
const SMALL_CALL_LIMIT: Duration = Duration::from_secs(3);
/// How late a stream frame or a mid-sized response may land behind another stream's large one.
const SHARE_LIMIT: Duration = Duration::from_millis(1500);
/// The server's 16 requests in flight at ~4 MiB each for these reads, with room; not the burst's 128 MiB.
const SERVER_GROWTH_LIMIT: u64 = 96 * MIB;
/// Pipelined reads from a reader that stopped reading.
const BURST: usize = 64;
/// Listings of `LISTING_FILES` long names, ~330 KB each: unbounded, they grow the server ~170 MB.
const CALL_BURST: usize = 512;
const LISTING_FILES: usize = 600;
/// Per second, while an upload fills the uplink: slower, since its credit shares that uplink, never stopped.
const UPLOAD_STREAM_FLOOR: u64 = 128 * KIB;
/// A busy terminal may slow another stream's turn this much, not starve it.
const SHARE_RATIO_LIMIT: f64 = 3.0;
/// Large reads in flight at once, as many as the server runs at a time.
const SLOT_READS: usize = 16;
/// A far link's first large read pays one more round trip than a nearby read plus the round
/// trip, while the window measures the link; a later one goes at that pace.
/// A first far read may take a nearby read plus this many round trips and the slack: its
/// request and answer cross, the window measures the link, and half a round trip spare.
const FIRST_FAR_READ_ROUNDS: f64 = 2.5;
const FIRST_FAR_READ_SLACK: Duration = Duration::from_millis(75);
/// Fresh connections each far read is taken on; the median counts, so one slow run can't fail it.
const FAR_READ_SAMPLES: usize = 3;
const LATER_FAR_READ_RATIO: f64 = 1.25;
const FAR_ONE_WAY: Duration = Duration::from_millis(50);
/// Slow enough that a window sized past the link holds a small call behind the read.
const CAPPED_BPS: u64 = 3 * MIB / 2;
/// A cell link before and after it drops.
const DROP_FROM_BPS: u64 = 4 * MIB;
const DROP_TO_BPS: u64 = 256 * KIB;
const STEPS: usize = 40;
const FLOOD: &str = "yes 0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";
const TICKER: &str = "while :; do echo tick; sleep 0.1; done";

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

async fn timed_read(client: &RpcClient, path: &str) -> Option<Duration> {
    let started = Instant::now();
    match tokio::time::timeout(IO, client.read_attachment(path.to_string())).await {
        Ok(Ok(_)) => Some(started.elapsed()),
        _ => None,
    }
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
    (n as f64 / MIB as f64 * 100.0).round() / 100.0
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn write_turn(sandbox: &Sandbox, steps: usize) {
    let mut out = String::new();
    for n in 0..steps {
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
    out += &result_line();
    std::fs::write(sandbox.root.join("turn.jsonl"), out).expect("write the turn script");
}

/// One turn whose single tool result is `bytes` long: one large session row.
fn write_big_turn(sandbox: &Sandbox, bytes: usize) {
    let mut out = String::new();
    for line in [
        json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":"reading a large file"},
            {"type":"tool_use","id":"toolu_%TURN%_big","name":"Read","input":{"file_path":"big.log"}}]},
            "session_id":"%SID%"}),
        json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"toolu_%TURN%_big","content":"x".repeat(bytes),"is_error":false}]},
            "session_id":"%SID%"}),
    ] {
        out += &line.to_string();
        out.push('\n');
    }
    out += &result_line();
    std::fs::write(sandbox.root.join("turn.jsonl"), out).expect("write the turn script");
}

fn result_line() -> String {
    let mut line = json!({"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%SID%",
        "num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4},
        "modelUsage":{"claude-e2e":{"contextWindow":200000}}})
    .to_string();
    line.push('\n');
    line
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
            with_meta: true,
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

/// `false` when the turn did not finish within `within`.
async fn until_turn_done(events: &mut SessionEvents, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(frame)) => {
                let event = match frame {
                    SessionFrame::Item(item) => item.event,
                    SessionFrame::Bare(event) => event,
                };
                if matches!(event, SessionEvent::TurnDone { .. }) {
                    return true;
                }
            }
            _ => return false,
        }
    }
}

async fn send_turn(client: &RpcClient, session: &str) {
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
}

/// Where the session's log ends, so a stream resumed there carries only new rows.
async fn end_cursor(client: &RpcClient, session: &str) -> LogCursor {
    let mut cursor = LogCursor::Empty;
    loop {
        let page = client
            .list_session_turns(session.to_string(), cursor, 2000)
            .await
            .expect("list_session_turns");
        cursor = page.next_cursor;
        if page.remaining == 0 {
            return cursor;
        }
    }
}

async fn resume_at_end(phone: &RpcClient, desktop: &RpcClient, session: &str) -> SessionEvents {
    let cursor = end_cursor(desktop, session).await;
    let (_, _, events) = phone
        .open_session_stream(StreamRequest::ResumeSession {
            session_id: session.to_string(),
            model: None,
            effort: None,
            permission_mode: None,
            cursor: Some(cursor),
            echo_user_messages: true,
            with_meta: true,
            all_partials: false,
        })
        .await
        .expect("resume_session");
    events
}

async fn save_blob(client: &RpcClient, ws: &str, name: &str, bytes: usize) -> String {
    let blob: Vec<u8> = (0..bytes).map(|i| (i * 31 % 251) as u8).collect();
    client
        .save_attachment(ws.to_string(), name.into(), STANDARD.encode(&blob))
        .await
        .expect("save_attachment")
}

/// `None` when the call failed or took longer than `IO`.
async fn timed_small_call(client: &RpcClient) -> Option<Duration> {
    let started = Instant::now();
    match tokio::time::timeout(IO, client.default_shell_name()).await {
        Ok(Ok(_)) => Some(started.elapsed()),
        _ => None,
    }
}

fn within(took: &[Option<Duration>], limit: Duration) -> bool {
    !took.is_empty() && took.iter().all(|t| t.is_some_and(|t| t < limit))
}

/// `None` when any timed out.
fn median(took: &[Option<Duration>]) -> Option<Duration> {
    let mut took: Vec<Duration> = took.iter().copied().collect::<Option<_>>()?;
    took.sort();
    took.get(took.len() / 2).copied()
}

fn millis(took: &[Option<Duration>]) -> Vec<Option<u64>> {
    took.iter().map(|t| t.map(ms)).collect()
}

/// When each output arrived, and how many bytes it carried.
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<(Instant, u64)>>>);

impl Output {
    fn follow(mut stream: TerminalStream) -> Self {
        let output = Self::default();
        let seen = output.0.clone();
        tokio::spawn(async move {
            while let Some(data) = next_output(&mut stream).await {
                seen.lock()
                    .unwrap()
                    .push((Instant::now(), data.len() as u64 * 3 / 4));
            }
        });
        output
    }

    fn bytes_between(&self, from: Instant, to: Instant) -> u64 {
        let seen = self.0.lock().unwrap();
        seen.iter()
            .filter(|(at, _)| *at >= from && *at < to)
            .map(|(_, n)| n)
            .sum()
    }

    fn first_after(&self, from: Instant) -> Option<Duration> {
        let seen = self.0.lock().unwrap();
        seen.iter()
            .find(|(at, _)| *at >= from)
            .map(|(at, _)| *at - from)
    }

    /// The longest silence between outputs from `from` to `to`, counting both ends.
    fn longest_gap(&self, from: Instant, to: Instant) -> Duration {
        let seen = self.0.lock().unwrap();
        let mut last = from;
        let mut gap = Duration::ZERO;
        for (at, _) in seen.iter().filter(|(at, _)| *at > from && *at <= to) {
            gap = gap.max(*at - last);
            last = *at;
        }
        gap.max(to.saturating_duration_since(last))
    }
}

async fn terminal(client: &RpcClient, command: &str) -> (String, Output) {
    let (id, _stream, output) = client
        .spawn_shell_terminal_stream(command.into(), 120, 32)
        .await
        .expect("spawn_shell_terminal");
    (id, Output::follow(output))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transport_fairness() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    write_turn(&sandbox, STEPS);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("transport_fairness", &sandbox);
    let server = Server::start(&sandbox).await;
    let desktop = server.client();
    let project = onboard_and_import(&mut j, &desktop, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &desktop, &project, "Fairness").await;
    let ws = text(&view, "/item/id");
    let two_mib = save_blob(&desktop, &ws, "two.bin", 1536 * 1024).await;
    let mid = save_blob(&desktop, &ws, "mid.bin", 20 * 1024).await;
    let large = save_blob(&desktop, &ws, "large.bin", 3584 * 1024).await;

    j.step("a stalled reader's pipelined large reads hold bounded server memory");
    {
        let pid = server.pid();
        // Deep, so the drain after the stall isn't paced a chunk at a time.
        let (phone, link) = phone(server.socket(), true).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let before = rss_bytes(pid);
        link.down_stalled.store(true, Relaxed);
        let mut reads = tokio::task::JoinSet::new();
        for _ in 0..BURST {
            let (phone, path) = (phone.clone(), two_mib.clone());
            reads.spawn(async move { phone.read_attachment(path).await.is_ok() });
        }
        let peak = peak_rss(pid, Duration::from_secs(8)).await;
        link.down_stalled.store(false, Relaxed);
        let mut answered = 0;
        let drained = tokio::time::timeout(IO, async {
            while let Some(read) = reads.join_next().await {
                answered += usize::from(read.unwrap_or(false));
            }
        })
        .await
        .is_ok();
        j.note(
            "burst_rss_mib",
            json!({ "reads": BURST, "before": mib(before), "peak": mib(peak), "growth": mib(peak.saturating_sub(before)) }),
        );
        j.guard(
            "server memory stays bounded while the reads wait on the stalled link",
            peak.saturating_sub(before) < SERVER_GROWTH_LIMIT,
            json!({ "reads": BURST, "limit_mib": mib(SERVER_GROWTH_LIMIT) }),
        );
        j.guard(
            "every read is answered once the link moves",
            drained && answered == BURST,
            json!({}),
        );
    }

    j.step("a stalled reader's pipelined small calls hold bounded server memory");
    {
        let listing = sandbox.root.join("listing");
        std::fs::create_dir_all(&listing).expect("listing dir");
        for i in 0..LISTING_FILES {
            std::fs::write(listing.join(format!("{i:04}-{}", "x".repeat(200))), b"")
                .expect("listing file");
        }
        let listing = listing.display().to_string();
        let pid = server.pid();
        let (phone, link) = phone(server.socket(), true).await;
        let bytes = phone
            .browse_directory(Some(listing.clone()))
            .await
            .map(|l| serde_json::to_vec(&l).map_or(0, |v| v.len()))
            .unwrap_or(0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let before = rss_bytes(pid);
        link.down_stalled.store(true, Relaxed);
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..CALL_BURST {
            let (phone, listing) = (phone.clone(), listing.clone());
            calls.spawn(async move { phone.browse_directory(Some(listing)).await.is_ok() });
        }
        let peak = peak_rss(pid, Duration::from_secs(8)).await;
        link.down_stalled.store(false, Relaxed);
        let mut answered = 0;
        let drained = tokio::time::timeout(IO, async {
            while let Some(call) = calls.join_next().await {
                answered += usize::from(call.unwrap_or(false));
            }
        })
        .await
        .is_ok();
        j.note(
            "call_burst_rss_mib",
            json!({ "calls": CALL_BURST, "answer_kib": bytes / 1024, "before": mib(before), "peak": mib(peak), "growth": mib(peak.saturating_sub(before)) }),
        );
        j.guard(
            "server memory stays bounded while the calls wait on the stalled link",
            peak.saturating_sub(before) < SERVER_GROWTH_LIMIT,
            json!({ "calls": CALL_BURST, "limit_mib": mib(SERVER_GROWTH_LIMIT) }),
        );
        j.guard(
            "every call is answered once the link moves",
            drained && answered == CALL_BURST,
            json!({}),
        );
    }

    j.step("a small call overtakes a large response the link is still carrying");
    {
        let (phone, link) = phone(server.socket(), false).await;
        let (ticker, _ticks) = terminal(&phone, TICKER).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        link.down_bps.store(FAST_BPS, Relaxed);
        let read = {
            let (phone, path) = (phone.clone(), large.clone());
            tokio::spawn(async move {
                let started = Instant::now();
                let ok = tokio::time::timeout(IO, phone.read_attachment(path))
                    .await
                    .is_ok_and(|read| read.is_ok());
                (ok, started.elapsed())
            })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut small = Vec::new();
        while !read.is_finished() && small.len() < 4 {
            small.push(timed_small_call(&phone).await);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let (large_ok, large_took) = read.await.expect("the large read");
        j.note(
            "small_behind_large_response",
            json!({ "link_kib_s": FAST_BPS / KIB, "large_ms": ms(large_took), "small_ms": millis(&small) }),
        );
        j.guard(
            "the large response takes longer than a small call may",
            large_ok && large_took > SMALL_CALL_LIMIT,
            json!({ "slower_than_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "every small call issued behind it answers in time",
            within(&small, SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT) }),
        );
        let _ = desktop.terminal_close(ticker).await;
    }

    j.step("a mid-sized response is not held behind a large one");
    {
        let (phone, link) = phone(server.socket(), false).await;
        link.down_bps.store(PHONE_BPS, Relaxed);
        let read = |path: String| {
            let phone = phone.clone();
            async move {
                let started = Instant::now();
                let ok = phone.read_attachment(path).await.is_ok();
                (ok, started.elapsed())
            }
        };
        let (_, alone) = read(mid.clone()).await;
        let big = tokio::spawn(read(two_mib.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (mid_ok, behind) = read(mid.clone()).await;
        let (big_ok, big_took) = big.await.expect("the large read");
        j.note(
            "mid_behind_large_response",
            json!({ "link_kib_s": PHONE_BPS / KIB, "mid_alone_ms": ms(alone), "mid_behind_ms": ms(behind), "large_ms": ms(big_took) }),
        );
        j.guard(
            "the mid-sized response lands soon after it is asked for",
            mid_ok && big_ok && behind < SHARE_LIMIT,
            json!({ "limit_ms": ms(SHARE_LIMIT) }),
        );
    }

    j.step("an upload holds neither calls, keystrokes nor stream credit");
    {
        let (phone, link) = phone(server.socket(), false).await;
        let (flood, flooded) = terminal(&phone, FLOOD).await;
        let (typing, echoes) = terminal(&phone, "cat").await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        link.up_bps.store(PHONE_BPS, Relaxed);
        let sample = Duration::from_secs(2);
        let before_from = Instant::now();
        tokio::time::sleep(sample).await;
        let before = flooded.bytes_between(before_from, Instant::now());
        let blob: Vec<u8> = (0..1536 * 1024).map(|i| (i * 7 % 253) as u8).collect();
        let upload = {
            let (phone, ws) = (phone.clone(), ws.clone());
            let data = STANDARD.encode(&blob);
            tokio::spawn(async move {
                let started = Instant::now();
                let ok = phone
                    .save_attachment(ws, "photo.bin".into(), data)
                    .await
                    .is_ok();
                (ok, started.elapsed())
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        let during_from = Instant::now();
        let key = {
            let (phone, typing) = (phone.clone(), typing.clone());
            tokio::spawn(async move {
                let _ = phone.terminal_write(typing, STANDARD.encode(b"x\n")).await;
            })
        };
        let small = timed_small_call(&phone).await;
        let _ = key.await;
        let rest = sample.saturating_sub(during_from.elapsed());
        tokio::time::sleep(rest).await;
        let during = flooded.bytes_between(during_from, during_from + sample);
        let echo = echoes.first_after(during_from);
        let (upload_ok, upload_took) = upload.await.expect("the upload");
        j.note(
            "upload",
            json!({
                "uplink_kib_s": PHONE_BPS / KIB,
                "upload_ms": ms(upload_took),
                "small_ms": small.map(ms),
                "echo_ms": echo.map(ms),
                "terminal_kib_before": before / KIB,
                "terminal_kib_during": during / KIB,
            }),
        );
        j.guard(
            "the upload takes longer than a small call may",
            upload_ok && upload_took > SMALL_CALL_LIMIT,
            json!({ "slower_than_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "a small call issued during the upload answers in time",
            within(&[small], SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "a keystroke typed during the upload echoes in time",
            echo.is_some_and(|echo| echo < SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "a stream keeps flowing during the upload, since its credit still goes up",
            before > 0 && during >= UPLOAD_STREAM_FLOOR * sample.as_secs(),
            json!({ "at_least_kib_s": UPLOAD_STREAM_FLOOR / KIB }),
        );
        let _ = desktop.terminal_close(flood).await;
        let _ = desktop.terminal_close(typing).await;
    }

    let (session, mut turns) = start_session(&desktop, &ws).await;
    assert!(until_turn_done(&mut turns, IO).await, "the first turn");
    announce(&desktop, "desktop").await;

    j.step("an interactive call is not held behind the server's large reads");
    {
        let blob = save_blob(&desktop, &ws, "slot.bin", 64 * 1024).await;
        let (phone, link) = phone(server.socket(), false).await;
        link.down_bps.store(PHONE_BPS, Relaxed);
        let reads: Vec<_> = (0..SLOT_READS)
            .map(|_| {
                let (phone, blob) = (phone.clone(), blob.clone());
                tokio::spawn(async move { timed_read(&phone, &blob).await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        let _ = tokio::time::timeout(
            IO,
            phone.respond_permission(session.clone(), "no-such-ask".into(), true, None),
        )
        .await;
        let permission = started.elapsed();
        let small = timed_small_call(&phone).await;
        let mut slowest = Duration::ZERO;
        for read in reads {
            slowest = slowest.max(read.await.ok().flatten().unwrap_or(IO));
        }
        j.note(
            "behind_large_reads",
            json!({ "reads": SLOT_READS, "link_kib_s": PHONE_BPS / KIB, "reads_ms": ms(slowest), "respond_permission_ms": ms(permission), "small_ms": small.map(ms) }),
        );
        j.guard(
            "the reads take longer than a small call may",
            slowest > SMALL_CALL_LIMIT,
            json!({ "slower_than_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "an answer to a permission ask and a small call both land in time",
            permission < SMALL_CALL_LIMIT && within(&[small], SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT) }),
        );
    }

    j.step("an upload into a buffering uplink holds neither answers, events nor echo");
    {
        let (phone, link) = far_phone(server.socket(), false, Duration::ZERO, true).await;
        announce(&phone, "phone").await;
        watch(&phone, &session).await;
        let (flood, _) = terminal(&phone, FLOOD).await;
        let (typing, echoes) = terminal(&phone, "cat").await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        link.up_bps.store(PHONE_BPS, Relaxed);
        let blob: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i * 13 % 251) as u8).collect();
        let upload = {
            let (phone, ws) = (phone.clone(), ws.clone());
            let data = STANDARD.encode(&blob);
            tokio::spawn(async move {
                let started = Instant::now();
                let ok = phone
                    .save_attachment(ws, "upload.bin".into(), data)
                    .await
                    .is_ok();
                (ok, started.elapsed())
            })
        };
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let during = Instant::now();
        let _ = phone
            .terminal_write(typing.clone(), STANDARD.encode(b"x\n"))
            .await;
        let small = timed_small_call(&phone).await;
        let turn_at = Instant::now();
        send_turn(&desktop, &session).await;
        assert!(until_turn_done(&mut turns, IO).await, "the desktop's turn");
        tokio::time::sleep(SMALL_CALL_LIMIT).await;
        let echo = echoes.first_after(during);
        let event = link.first_event_after(turn_at);
        let (upload_ok, upload_took) = upload.await.expect("the upload");
        j.note(
            "upload_into_a_buffering_uplink",
            json!({
                "uplink_kib_s": PHONE_BPS / KIB,
                "uplink_buffer_mib": mib(DEEP_UPLINK_BYTES),
                "upload_ms": ms(upload_took),
                "small_ms": small.map(ms),
                "echo_ms": echo.map(ms),
                "event_ms": event.map(ms),
            }),
        );
        j.guard(
            "the upload takes longer than a small call may",
            upload_ok && upload_took > SMALL_CALL_LIMIT,
            json!({ "slower_than_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "a small call, an echo and an event all land in time meanwhile",
            within(&[small], SMALL_CALL_LIMIT)
                && echo.is_some_and(|echo| echo < SMALL_CALL_LIMIT)
                && event.is_some_and(|event| event < SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT) }),
        );
        let _ = desktop.terminal_close(flood).await;
        let _ = desktop.terminal_close(typing).await;
    }

    j.step("a busy terminal leaves another stream its share of the link");
    {
        let (phone, link) = phone(server.socket(), false).await;
        announce(&phone, "phone").await;
        watch(&phone, &session).await;
        let mut events = resume_at_end(&phone, &desktop, &session).await;
        link.down_bps.store(PHONE_BPS, Relaxed);
        let mut took = Vec::new();
        for busy in [false, true] {
            let flood = if busy {
                let (id, _) = terminal(&phone, FLOOD).await;
                tokio::time::sleep(Duration::from_secs(2)).await;
                Some(id)
            } else {
                None
            };
            let started = Instant::now();
            send_turn(&desktop, &session).await;
            assert!(until_turn_done(&mut turns, IO).await, "the desktop's turn");
            let reached = until_turn_done(&mut events, IO).await;
            took.push(reached.then(|| started.elapsed()));
            if let Some(id) = flood {
                let _ = desktop.terminal_close(id).await;
            }
        }
        let ratio = match (took[0], took[1]) {
            (Some(alone), Some(busy)) => Some(busy.as_secs_f64() / alone.as_secs_f64()),
            _ => None,
        };
        j.note(
            "turn_beside_a_busy_terminal",
            json!({ "link_kib_s": PHONE_BPS / KIB, "alone_ms": took[0].map(ms), "busy_ms": took[1].map(ms), "ratio": ratio }),
        );
        j.guard(
            "the turn reaches the phone with a busy terminal beside it",
            ratio.is_some(),
            json!({}),
        );
        j.guard(
            "the busy terminal slows it by a share, not a multiple",
            ratio.is_some_and(|r| r <= SHARE_RATIO_LIMIT),
            json!({ "at_most": SHARE_RATIO_LIMIT }),
        );
    }

    j.step("one stream's large frame does not hold the others");
    {
        write_big_turn(&sandbox, 1536 * 1024);
        let (phone, link) = phone(server.socket(), false).await;
        announce(&phone, "phone").await;
        watch(&phone, &session).await;
        let mut events = resume_at_end(&phone, &desktop, &session).await;
        let (ticker, ticks) = terminal(&phone, TICKER).await;
        let (typing, echoes) = terminal(&phone, "cat").await;
        link.down_bps.store(PHONE_BPS, Relaxed);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let started = Instant::now();
        send_turn(&desktop, &session).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let key_sent = Instant::now();
        let _ = phone
            .terminal_write(typing.clone(), STANDARD.encode(b"x\n"))
            .await;
        let reached = until_turn_done(&mut events, IO).await;
        let done = Instant::now();
        assert!(until_turn_done(&mut turns, IO).await, "the desktop's turn");
        let gap = ticks.longest_gap(started, done);
        let echo = echoes.first_after(key_sent);
        j.note(
            "beside_a_large_row",
            json!({
                "link_kib_s": PHONE_BPS / KIB,
                "turn_ms": ms(done - started),
                "longest_tick_gap_ms": ms(gap),
                "echo_ms": echo.map(ms),
            }),
        );
        j.guard(
            "the large row reaches the phone",
            reached && done - started > SHARE_LIMIT,
            json!({ "slower_than_ms": ms(SHARE_LIMIT) }),
        );
        j.guard(
            "a ticking terminal keeps ticking meanwhile",
            gap < SHARE_LIMIT,
            json!({ "limit_ms": ms(SHARE_LIMIT) }),
        );
        j.guard(
            "a keystroke echoes meanwhile",
            echo.is_some_and(|echo| echo < SHARE_LIMIT),
            json!({ "limit_ms": ms(SHARE_LIMIT) }),
        );
        let _ = desktop.terminal_close(ticker).await;
        let _ = desktop.terminal_close(typing).await;
    }

    j.step("a buffering link holds little ahead of a small call");
    {
        let mut report = Vec::new();
        let mut all = Vec::new();
        for (streams, bps) in [(3usize, PHONE_BPS), (1, SLOW_BPS)] {
            let (phone, link) = phone(server.socket(), true).await;
            link.down_bps.store(bps, Relaxed);
            let mut floods = Vec::new();
            for _ in 0..streams {
                floods.push(terminal(&phone, FLOOD).await.0);
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
            let buffered = link.buffered.load(Relaxed);
            let mut small = Vec::new();
            for _ in 0..3 {
                small.push(timed_small_call(&phone).await);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            report.push(json!({
                "streams": streams,
                "link_kib_s": bps / KIB,
                "buffered_kib": buffered / KIB,
                "small_ms": millis(&small),
            }));
            all.extend(small);
            for id in floods {
                let _ = desktop.terminal_close(id).await;
            }
        }
        j.note("buffering_link", json!(report));
        j.guard(
            "every small call answers in time with busy streams filling the link",
            within(&all, SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT), "link_buffer_mib": mib(DEEP_LINK_BYTES) }),
        );
    }

    let far_blob = save_blob(&desktop, &ws, "far.bin", 4 * 1024 * 1024).await;

    j.step("a far link with room carries a large read near a nearby one's pace");
    {
        let nearby = server.connect().await;
        let mut near = Vec::new();
        for _ in 0..3 {
            near.push(timed_read(&nearby, &far_blob).await);
        }
        let near_pace = median(&near);
        let mut report = Vec::new();
        let (mut first_in_pace, mut later_in_pace) = (near_pace.is_some(), near_pace.is_some());
        let mut measured_sooner = false;
        let ratio = |took: Option<Duration>, rtt: Duration| {
            took.zip(near_pace)
                .map(|(took, near)| took.as_secs_f64() / (near + rtt).as_secs_f64())
        };
        for rtt in [Duration::from_millis(20), FAR_ONE_WAY * 2] {
            let (mut cold, mut first, mut again) = (Vec::new(), Vec::new(), Vec::new());
            for _ in 0..FAR_READ_SAMPLES {
                // Without two agreeing small calls first there is no bare round trip to
                // measure a burst against, as a client's first calls on connecting give one.
                let fresh = uncapped(server.socket(), rtt / 2).await;
                cold.push(timed_read(&fresh, &far_blob).await);
                let client = uncapped(server.socket(), rtt / 2).await;
                let _ = timed_small_call(&client).await;
                let _ = timed_small_call(&client).await;
                first.push(timed_read(&client, &far_blob).await);
                again.push(timed_read(&client, &far_blob).await);
            }
            let (cold_pace, first_pace, again_pace) =
                (median(&cold), median(&first), median(&again));
            first_in_pace &= matches!(
                (first_pace, near_pace),
                (Some(first), Some(near))
                    if first <= near + rtt.mul_f64(FIRST_FAR_READ_ROUNDS) + FIRST_FAR_READ_SLACK
            );
            later_in_pace &= ratio(again_pace, rtt).is_some_and(|r| r <= LATER_FAR_READ_RATIO);
            // Where the window holds the read back, the measured round trip saves one; CPU
            // noise slows both alike.
            if rtt == FAR_ONE_WAY * 2 {
                measured_sooner = matches!(
                    (first_pace, cold_pace),
                    (Some(first), Some(cold)) if first + rtt / 2 <= cold
                );
            }
            report.push(json!({
                "rtt_ms": ms(rtt),
                "cold_first_ms": millis(&cold),
                "first_ms": millis(&first),
                "again_ms": millis(&again),
                "first_ratio": ratio(first_pace, rtt),
                "again_ratio": ratio(again_pace, rtt),
            }));
        }
        j.note(
            "far_read",
            json!({ "bytes_b64": 4 * MIB * 4 / 3, "near_ms": millis(&near), "far": report }),
        );
        j.guard(
            "a connection's first large read after its first small calls costs a round trip and a half more at most",
            first_in_pace,
            json!({ "rounds": FIRST_FAR_READ_ROUNDS, "slack_ms": ms(FIRST_FAR_READ_SLACK), "samples": FAR_READ_SAMPLES }),
        );
        j.guard(
            "on the far link, those small calls bring the first read half a round trip sooner at least",
            measured_sooner,
            json!({ "rtt_ms": ms(FAR_ONE_WAY * 2) }),
        );
        j.guard(
            "its next large read goes at a nearby read's pace plus the round trip",
            later_in_pace,
            json!({ "at_most": LATER_FAR_READ_RATIO }),
        );
    }

    j.step("a far link the window has measured keeps a small call ahead of a large read");
    {
        let (phone, link) = far_phone(server.socket(), true, FAR_ONE_WAY, false).await;
        link.down_bps.store(CAPPED_BPS, Relaxed);
        // As on the far link above, so the window measures this burst too.
        let _ = timed_small_call(&phone).await;
        let _ = timed_small_call(&phone).await;
        let read = {
            let (phone, blob) = (phone.clone(), far_blob.clone());
            tokio::spawn(async move { timed_read(&phone, &blob).await })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut small = Vec::new();
        for _ in 0..3 {
            small.push(timed_small_call(&phone).await);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let read = read.await.expect("the large read");
        j.note(
            "far_capped",
            json!({ "rtt_ms": ms(FAR_ONE_WAY * 2), "link_kib_s": CAPPED_BPS / KIB, "read_ms": read.map(ms), "small_ms": millis(&small) }),
        );
        j.guard(
            "the read takes longer than a small call may",
            read.is_some_and(|read| read > SMALL_CALL_LIMIT),
            json!({ "slower_than_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "a small call during it lands soon, not after it",
            within(&small, SHARE_LIMIT),
            json!({ "limit_ms": ms(SHARE_LIMIT) }),
        );
    }

    // Raw floods fill the link window, so they test its cut; frames keep one frame on the link.
    for (floods_of, features, note) in [
        ("raw", RAW_DESKTOP, "rate_drop"),
        ("frame", Features::ALL, "rate_drop_frames"),
    ] {
        j.step(&format!(
            "a small call after the link drops its rate still answers in time, beside {floods_of} floods"
        ));
        let (phone, link) =
            far_phone_announcing(server.socket(), true, FAR_ONE_WAY, false, Some(features)).await;
        link.down_bps.store(DROP_FROM_BPS, Relaxed);
        // Enough streams that their credit together outruns the window, so the window bounds the wait.
        let mut floods = Vec::new();
        for _ in 0..3 {
            floods.push(terminal(&phone, FLOOD).await);
        }
        tokio::time::sleep(Duration::from_secs(4)).await;
        let sample = Duration::from_secs(2);
        let from = Instant::now();
        tokio::time::sleep(sample).await;
        let until = Instant::now();
        let fast = floods
            .iter()
            .map(|(_, output)| output.bytes_between(from, until))
            .sum::<u64>()
            / sample.as_secs();
        link.down_bps.store(DROP_TO_BPS, Relaxed);
        let buffered = link.buffered.load(Relaxed);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut small = Vec::new();
        for _ in 0..4 {
            small.push(timed_small_call(&phone).await);
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        j.note(
            note,
            json!({
                "rtt_ms": ms(FAR_ONE_WAY * 2),
                "from_kib_s": DROP_FROM_BPS / KIB,
                "to_kib_s": DROP_TO_BPS / KIB,
                "stream_kib_s_before": fast / KIB,
                "buffered_kib_at_drop": buffered / KIB,
                "small_ms": millis(&small),
            }),
        );
        if features == RAW_DESKTOP {
            j.guard(
                "the streams used most of the fast link first",
                fast * 4 >= DROP_FROM_BPS * 3,
                json!({ "at_least_kib_s": DROP_FROM_BPS * 3 / 4 / KIB }),
            );
        }
        j.guard(
            "every small call after the drop answers in time",
            within(&small, SMALL_CALL_LIMIT),
            json!({ "limit_ms": ms(SMALL_CALL_LIMIT) }),
        );
        j.guard(
            "once the window has cut, small calls land soon",
            within(&small[2..], SHARE_LIMIT),
            json!({ "limit_ms": ms(SHARE_LIMIT) }),
        );
        for (id, _) in floods {
            let _ = desktop.terminal_close(id).await;
        }
    }

    j.finish();
}
