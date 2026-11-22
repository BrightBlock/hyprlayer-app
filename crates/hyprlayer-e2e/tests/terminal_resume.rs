//! A terminal costs a raw desktop what it prints: raw bytes on the wire, a reattach
//! from the client's offset replays nothing it already has, and a released
//! desktop still reads base64 in JSON. A frame desktop's resume is `shared_terminal_screen`'s.
#![cfg(unix)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_e2e::{json, Journey, Sandbox, Server, Value, RAW_DESKTOP};
use hyprlayer_proto::frame::{decode_stream_frame, encode_control, Decoder, Frame};
use hyprlayer_server::client::{RpcClient, TerminalStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const IO: Duration = Duration::from_secs(30);
/// How long a stream must stay silent to count as done replaying.
const QUIET: Duration = Duration::from_millis(800);
/// More than the ring holds, so offset 0 is trimmed by the time it finishes.
const PRINT: &str = "seq 1 60000; echo DONE; sleep 120";
const RATIO_LIMIT: f64 = 1.05;
/// A resume with nothing new costs its response and nothing more.
const QUIET_RESUME_LIMIT: u64 = 512;
const RESUME_TAIL: u64 = 1000;

/// Server-to-client bytes on one connection, as a proxy between the two sees them.
#[derive(Default)]
struct Link {
    down: AtomicU64,
    tap: Mutex<Tap>,
}

#[derive(Default)]
struct Tap {
    decoder: Decoder,
    data_frames: u64,
}

impl Link {
    fn down(&self) -> u64 {
        self.down.load(Relaxed)
    }
    fn data_frames(&self) -> u64 {
        self.tap.lock().unwrap().data_frames
    }
}

async fn counted(socket: &Path) -> (Arc<RpcClient>, Arc<Link>) {
    let unix = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (mut srv_rd, mut srv_wr) = unix.into_split();
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (mut cli_rd, mut cli_wr) = tokio::io::split(proxy_end);
    let link = Arc::new(Link::default());
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut cli_rd, &mut srv_wr).await;
    });
    let down = link.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match srv_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            down.down.fetch_add(n as u64, Relaxed);
            {
                let mut tap = down.tap.lock().unwrap();
                tap.decoder.feed(&buf[..n]);
                while let Ok(Some(frame)) = tap.decoder.next_frame() {
                    if matches!(frame, Frame::Data(_)) {
                        tap.data_frames += 1;
                    }
                }
            }
            if cli_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    });
    let client = RpcClient::connect_announcing(client_end, RAW_DESKTOP)
        .await
        .expect("handshake");
    (client, link)
}

/// What one stream delivered before it went quiet.
#[derive(Default)]
struct Read {
    raw: u64,
    /// The first output's offset, and where the last one ended.
    first: Option<u64>,
    next: Option<u64>,
    /// Bytes the same events take as `TerminalEvent` JSON, as the desktop hands them to its webview.
    json: u64,
    text: Vec<u8>,
}

impl Read {
    fn take(&mut self, event: TerminalEvent) {
        self.json += serde_json::to_vec(&event).unwrap().len() as u64;
        if let TerminalEvent::Output { data, offset, .. } = event {
            let bytes = STANDARD.decode(data).expect("output is base64");
            self.first = self.first.or(offset);
            self.next = offset.map(|at| at + bytes.len() as u64);
            self.raw += bytes.len() as u64;
            self.text.extend_from_slice(&bytes);
        }
    }

    fn saw(&self, needle: &[u8]) -> bool {
        self.text.windows(needle.len()).any(|w| w == needle)
    }
}

async fn until(events: &mut TerminalStream, needle: &[u8]) -> Read {
    let mut read = Read::default();
    while !read.saw(needle) {
        match tokio::time::timeout(IO, events.recv()).await {
            Ok(Some(event)) => read.take(event),
            _ => break,
        }
    }
    read
}

async fn until_quiet(events: &mut TerminalStream) -> Read {
    let mut read = Read::default();
    while let Ok(Some(event)) = tokio::time::timeout(QUIET, events.recv()).await {
        read.take(event);
    }
    read
}

/// A released desktop's side of the wire: a hello with no features, then one stream request.
async fn released_desktop_output(socket: &Path) -> Vec<u8> {
    let mut sock = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let hello = r#"{"hello":{"protocol_version":1}}"#;
    let open = r#"{"stream_request":{"id":1,"body":{"spawn_shell_terminal":{"command":"echo LEGACY_FORM; sleep 120","cols":80,"rows":24}}}}"#;
    sock.write_all(&encode_control(hello)).await.unwrap();
    sock.write_all(&encode_control(open)).await.unwrap();
    let mut decoder = Decoder::new();
    let mut buf = vec![0u8; 16 * 1024];
    let mut seen = Vec::new();
    let read = async {
        loop {
            let n = sock.read(&mut buf).await.expect("read the server");
            assert!(n > 0, "the server closed the connection");
            decoder.feed(&buf[..n]);
            while let Some(frame) = decoder.next_frame().expect("frames decode") {
                if let Frame::Data(payload) = frame {
                    let (_, event) = decode_stream_frame(&payload).expect("a stream frame");
                    seen.push(event.to_vec());
                    if String::from_utf8_lossy(event).contains("output") {
                        return;
                    }
                }
            }
        }
    };
    let _ = tokio::time::timeout(IO, read).await;
    seen.pop().unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_resume() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("terminal_resume", &sandbox);
    let server = Server::start(&sandbox).await;
    let (client, link) = counted(server.socket()).await;

    j.step("a printing terminal crosses the wire as raw bytes");
    let (down0, frames0) = (link.down(), link.data_frames());
    let (terminal, _, mut live) = client
        .spawn_shell_terminal_stream(PRINT.into(), 120, 40)
        .await
        .expect("spawn the printing terminal");
    let printed = until(&mut live, b"DONE").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let wire = link.down() - down0;
    let ratio = wire as f64 / printed.raw.max(1) as f64;
    j.note(
        "live",
        json!({
            "raw_bytes": printed.raw,
            "wire_bytes": wire,
            "data_frames": link.data_frames() - frames0,
            "wire_ratio": ratio,
            "webview_json_bytes": printed.json,
            "webview_ratio": printed.json as f64 / printed.raw.max(1) as f64,
        }),
    );
    j.guard(
        "the terminal printed past the ring",
        printed.saw(b"DONE") && printed.raw > 256 * 1024,
        json!({}),
    );
    j.guard(
        "the wire carries little more than the output",
        ratio <= RATIO_LIMIT,
        json!({ "limit": RATIO_LIMIT }),
    );
    j.guard(
        "every output says where it starts",
        printed.next.is_some(),
        json!({}),
    );
    drop(live);
    let end = printed.next.unwrap_or_default();

    j.step("a reattach from where the client left off replays nothing");
    let mut quiet = Vec::new();
    for _ in 0..3 {
        let before = link.down();
        let attached = client
            .terminal_attach_stream(terminal.clone(), Some(end))
            .await
            .expect("terminal_attach")
            .expect("a live terminal");
        let mut events = attached.events;
        let replay = until_quiet(&mut events).await;
        quiet.push(json!({
            "resumed": attached.resumed,
            "raw_bytes": replay.raw,
            "wire_bytes": link.down() - before,
        }));
    }
    j.note("quiet_resumes", Value::Array(quiet.clone()));
    j.guard(
        "the server resumes an offset it still holds",
        quiet.iter().all(|r| r["resumed"] == json!(true)),
        json!({}),
    );
    j.guard(
        "a resume with no new output costs about nothing",
        quiet.iter().all(|r| {
            r["raw_bytes"] == json!(0)
                && r["wire_bytes"]
                    .as_u64()
                    .is_some_and(|w| w < QUIET_RESUME_LIMIT)
        }),
        json!({ "limit_bytes": QUIET_RESUME_LIMIT }),
    );

    let before = link.down();
    let attached = client
        .terminal_attach_stream(terminal.clone(), Some(end.saturating_sub(RESUME_TAIL)))
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let mut events = attached.events;
    let tail = until_quiet(&mut events).await;
    j.note(
        "tail_resume",
        json!({ "raw_bytes": tail.raw, "wire_bytes": link.down() - before }),
    );
    j.guard(
        "a resume replays only what follows its offset",
        attached.resumed
            && tail.raw == RESUME_TAIL
            && tail.first == Some(end.saturating_sub(RESUME_TAIL))
            && tail.saw(b"DONE"),
        json!({ "tail_bytes": RESUME_TAIL }),
    );

    j.step("an offset the ring no longer holds is reported");
    let before = link.down();
    let attached = client
        .terminal_attach_stream(terminal.clone(), Some(0))
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let mut events = attached.events;
    let trimmed = until_quiet(&mut events).await;
    let trimmed_wire = link.down() - before;
    j.guard(
        "the server says it did not resume",
        !attached.resumed,
        json!({}),
    );
    j.guard(
        "it replays the scrollback it holds, from where that starts",
        trimmed.first.is_some_and(|first| first > 0)
            && trimmed.next == Some(end)
            && trimmed.saw(b"DONE"),
        json!({}),
    );

    let before = link.down();
    let attached = client
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let mut events = attached.events;
    let fresh = until_quiet(&mut events).await;
    j.note(
        "replays",
        json!({
            "trimmed_offset": { "raw_bytes": trimmed.raw, "wire_bytes": trimmed_wire },
            "no_offset": { "raw_bytes": fresh.raw, "wire_bytes": link.down() - before },
        }),
    );
    j.guard(
        "an attach without an offset replays the scrollback as before",
        !attached.resumed && fresh.raw == trimmed.raw && fresh.first == trimmed.first,
        json!({}),
    );

    j.step("a client that never announced raw_terminal still reads base64 in JSON");
    let event = released_desktop_output(server.socket()).await;
    let parsed: Value = serde_json::from_slice(&event).unwrap_or_default();
    let text = parsed["data"]
        .as_str()
        .and_then(|data| STANDARD.decode(data).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    j.guard(
        "its output is the JSON it always was",
        parsed["kind"] == json!("output")
            && parsed.as_object().is_some_and(|o| o.len() == 2)
            && text.contains("LEGACY_FORM"),
        json!({}),
    );

    j.finish();
}
