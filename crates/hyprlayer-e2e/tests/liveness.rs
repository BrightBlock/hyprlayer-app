//! A link that goes silent without closing is noticed on both ends within
//! `NOTICED_WITHIN`, while an idle link, a slow and busy one, and a client
//! from before the heartbeat all stay connected past that deadline.
#![cfg(unix)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_proto::frame::{encode_control, Decoder, Frame};
use hyprlayer_proto::PROTOCOL_VERSION;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::rpc::WatchedSession;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const KIB: u64 = 1024;
/// The 30 s deadline, plus a viewer poll and a loaded host.
const NOTICED_WITHIN: Duration = Duration::from_secs(40);
/// Longer than the deadline, so a link mistaken for dead is gone by then.
const QUIET_FOR: Duration = Duration::from_secs(45);
/// A 2G-like downlink, with a terminal printing as fast as it can.
const BUSY_BPS: u64 = 16 * KIB;
const BUSY_GAP_LIMIT: Duration = Duration::from_secs(5);
const SMALL_CALL_LIMIT: Duration = Duration::from_secs(10);
const IO: Duration = Duration::from_secs(60);
const FLOOD: &str = "yes 0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";

const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]},"session_id":"%s"}\n' "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"ok","session_id":"%s","num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1},"modelUsage":{"claude-e2e":{"contextWindow":200000}}}\n' "$sid"
done
"#;

/// One connection through a proxy that can hold both directions or throttle the downlink.
#[derive(Default)]
struct Link {
    /// Neither direction reads or delivers, and nothing closes.
    silent: AtomicBool,
    down_bps: AtomicU64,
}

async fn pipe<R, W>(mut rd: R, mut wr: W, link: Arc<Link>, down: bool)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 4096];
    loop {
        while link.silent.load(Relaxed) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let n = match rd.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        while link.silent.load(Relaxed) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if wr.write_all(&buf[..n]).await.is_err() {
            break;
        }
        let bps = if down { link.down_bps.load(Relaxed) } else { 0 };
        if bps > 0 {
            tokio::time::sleep(Duration::from_secs_f64(n as f64 / bps as f64)).await;
        }
    }
}

async fn phone(socket: &Path) -> (Arc<RpcClient>, Arc<Link>) {
    let unix = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (srv_rd, srv_wr) = unix.into_split();
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (cli_rd, cli_wr) = tokio::io::split(proxy_end);
    let link = Arc::new(Link::default());
    tokio::spawn(pipe(cli_rd, srv_wr, link.clone(), false));
    tokio::spawn(pipe(srv_rd, cli_wr, link.clone(), true));
    let client = RpcClient::connect(client_end).await.expect("handshake");
    (client, link)
}

/// When the client's runtime events ended: how the desktop learns its connection is gone.
fn events_end(client: &RpcClient) -> Arc<Mutex<Option<Instant>>> {
    let ended = Arc::new(Mutex::new(None));
    let mut events = client.take_events().expect("runtime events");
    let mark = ended.clone();
    tokio::spawn(async move {
        while events.recv().await.is_some() {}
        *mark.lock().unwrap() = Some(Instant::now());
    });
    ended
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

async fn watch(client: &RpcClient, session: &str) -> Vec<String> {
    client
        .watch_sessions(vec![WatchedSession {
            session_id: session.to_string(),
            rev: 0,
        }])
        .await
        .expect("watch_sessions")
        .first()
        .map(|rev| rev.viewers.iter().map(|v| v.device.clone()).collect())
        .unwrap_or_default()
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

async fn timed_small_call(client: &RpcClient) -> Option<Duration> {
    let started = Instant::now();
    match tokio::time::timeout(IO, client.default_shell_name()).await {
        Ok(Ok(_)) => Some(started.elapsed()),
        _ => None,
    }
}

fn secs(d: Duration) -> f64 {
    (d.as_secs_f64() * 10.0).round() / 10.0
}

/// A client from before the heartbeat: a bare hello, then raw requests, read by a task that only drains.
struct OldClient {
    writer: tokio::net::unix::OwnedWriteHalf,
    answered: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}

impl OldClient {
    async fn connect(socket: &Path) -> Self {
        let unix = tokio::net::UnixStream::connect(socket)
            .await
            .expect("connect to the sandbox socket");
        let (mut rd, mut writer) = unix.into_split();
        let hello = json!({ "hello": { "protocol_version": PROTOCOL_VERSION } }).to_string();
        writer
            .write_all(&encode_control(&hello))
            .await
            .expect("send the hello");
        let answered = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        {
            let (answered, closed) = (answered.clone(), closed.clone());
            tokio::spawn(async move {
                let mut decoder = Decoder::new();
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    let n = match rd.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    decoder.feed(&buf[..n]);
                    while let Ok(Some(frame)) = decoder.next_frame() {
                        if let Frame::Control(line) = frame {
                            if line.starts_with(r#"{"response":"#) {
                                answered.fetch_add(1, Relaxed);
                            }
                        }
                    }
                }
                closed.store(true, Relaxed);
            });
        }
        Self {
            writer,
            answered,
            closed,
        }
    }

    async fn send(&mut self, id: u64, request: Request) {
        let line =
            json!({ "request": { "id": id, "body": serde_json::to_value(&request).unwrap() } })
                .to_string();
        self.writer
            .write_all(&encode_control(&line))
            .await
            .expect("send a request");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn liveness() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("liveness", &sandbox);
    let server = Server::start(&sandbox).await;
    let desktop = server.client();
    let project = onboard_and_import(&mut j, &desktop, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &desktop, &project, "Liveness").await;
    let ws = text(&view, "/item/id");
    let (session, mut turns) = start_session(&desktop, &ws).await;
    until_turn_done(&mut turns).await;
    announce(&desktop, "desktop").await;

    let (dead, dead_link) = phone(server.socket()).await;
    let dead_events = events_end(&dead);
    announce(&dead, "dead").await;
    watch(&dead, &session).await;

    let (idle, _idle_link) = phone(server.socket()).await;
    let idle_events = events_end(&idle);
    announce(&idle, "idle").await;
    watch(&idle, &session).await;

    let (busy, busy_link) = phone(server.socket()).await;
    let busy_events = events_end(&busy);
    announce(&busy, "busy").await;
    let (flood, _stream, mut output) = busy
        .spawn_shell_terminal_stream(FLOOD.into(), 120, 32)
        .await
        .expect("spawn_shell_terminal");
    let arrivals = Arc::new(Mutex::new(Vec::<Instant>::new()));
    {
        let arrivals = arrivals.clone();
        tokio::spawn(async move {
            while let Some(TerminalEvent::Output { .. }) = output.recv().await {
                arrivals.lock().unwrap().push(Instant::now());
            }
        });
    }
    busy_link.down_bps.store(BUSY_BPS, Relaxed);

    let mut old = OldClient::connect(server.socket()).await;
    old.send(
        1,
        Request::AnnounceClient {
            client_id: "old".into(),
            device: "device-old".into(),
            capabilities: TARGETED_EVENTS.map(str::to_string).to_vec(),
        },
    )
    .await;
    old.send(
        2,
        Request::WatchSessions {
            sessions: vec![WatchedSession {
                session_id: session.clone(),
                rev: 0,
            }],
        },
    )
    .await;
    let everyone = watch(&desktop, &session).await;

    j.step("a link that goes silent both ways is noticed on both ends");
    dead_link.silent.store(true, Relaxed);
    let silent_from = Instant::now();
    let pending = {
        let dead = dead.clone();
        tokio::spawn(async move {
            let outcome = dead.default_shell_name().await;
            (outcome.err().map(|e| e.code), silent_from.elapsed())
        })
    };
    let mut dropped_at = None;
    while silent_from.elapsed() < QUIET_FOR {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if dropped_at.is_none()
            && !watch(&desktop, &session)
                .await
                .contains(&"device-dead".into())
        {
            dropped_at = Some(silent_from.elapsed());
        }
    }
    let call = tokio::time::timeout(Duration::from_millis(100), pending)
        .await
        .ok()
        .and_then(Result::ok);
    let events_ended = dead_events.lock().unwrap().map(|at| at - silent_from);
    j.note(
        "dead_link_s",
        json!({
            "call_failed_after": call.as_ref().map(|(_, at)| secs(*at)),
            "events_ended_after": events_ended.map(secs),
            "server_dropped_viewer_after": dropped_at.map(secs),
        }),
    );
    j.guard(
        "the call pending on it fails as a disconnect",
        call.as_ref().is_some_and(|(code, at)| {
            code.as_deref() == Some("server_disconnected") && *at < NOTICED_WITHIN
        }),
        json!({ "within_s": NOTICED_WITHIN.as_secs() }),
    );
    j.guard(
        "its runtime events end, so the desktop reconnects",
        events_ended.is_some_and(|at| at < NOTICED_WITHIN),
        json!({ "within_s": NOTICED_WITHIN.as_secs() }),
    );
    j.guard(
        "the server stops listing it as a viewer",
        everyone.contains(&"device-dead".into())
            && dropped_at.is_some_and(|at| at < NOTICED_WITHIN),
        json!({ "within_s": NOTICED_WITHIN.as_secs() }),
    );

    j.step("an idle link outlives the deadline");
    let idle_call = timed_small_call(&idle).await;
    let viewers = watch(&desktop, &session).await;
    j.guard(
        "a call on it still answers, on the same connection",
        idle_call.is_some() && idle_events.lock().unwrap().is_none(),
        json!({}),
    );
    j.guard(
        "the server still lists it as a viewer",
        viewers.contains(&"device-idle".into()),
        json!({}),
    );

    j.step("a slow, busy link outlives the deadline");
    let busy_call = timed_small_call(&busy).await;
    let longest_gap = {
        let arrivals = arrivals.lock().unwrap();
        arrivals
            .windows(2)
            .filter(|w| w[0] >= silent_from)
            .map(|w| w[1] - w[0])
            .max()
    };
    j.note(
        "busy_link",
        json!({
            "link_kib_s": BUSY_BPS / KIB,
            "small_call_s": busy_call.map(secs),
            "longest_output_gap_s": longest_gap.map(secs),
        }),
    );
    j.guard(
        "its terminal keeps printing to it",
        longest_gap.is_some_and(|gap| gap < BUSY_GAP_LIMIT),
        json!({ "gap_under_s": BUSY_GAP_LIMIT.as_secs() }),
    );
    j.guard(
        "a call on it still answers, on the same connection",
        busy_call.is_some_and(|took| took < SMALL_CALL_LIMIT)
            && busy_events.lock().unwrap().is_none(),
        json!({ "within_s": SMALL_CALL_LIMIT.as_secs() }),
    );
    let _ = desktop.terminal_close(flood).await;

    j.step("a client from before the heartbeat keeps today's behaviour");
    j.guard(
        "it stays connected through the silence, and still a viewer",
        old.answered.load(Relaxed) == 2
            && !old.closed.load(Relaxed)
            && viewers.contains(&"device-old".into()),
        json!({ "answered": old.answered.load(Relaxed) }),
    );

    j.finish();
}
