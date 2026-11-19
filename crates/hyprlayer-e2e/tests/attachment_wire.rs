//! What an attachment costs on the wire, counted at the server socket: a remote
//! client on a metered link pays for every byte of its encoding.
#![cfg(unix)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Sandbox, Server};
use hyprlayer_server::client::RpcClient;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const PAYLOAD: usize = 1024 * 1024;
/// base64's 4/3, plus the request envelope.
const MAX_RATIO: f64 = 1.4;

#[derive(Default)]
struct Wire {
    up: AtomicU64,
    down: AtomicU64,
}

/// A client whose bytes to and from the server socket are counted.
async fn counted_client(socket: &Path) -> (Arc<RpcClient>, Arc<Wire>) {
    let server = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (server_rd, server_wr) = server.into_split();
    let (client_rd, client_wr) = tokio::io::split(proxy_end);
    let wire = Arc::new(Wire::default());
    tokio::spawn(relay(server_rd, client_wr, wire.clone(), true));
    tokio::spawn(relay(client_rd, server_wr, wire.clone(), false));
    let client = RpcClient::connect(client_end).await.expect("handshake");
    (client, wire)
}

async fn relay<R, W>(mut rd: R, mut wr: W, wire: Arc<Wire>, down: bool)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match rd.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let counter = if down { &wire.down } else { &wire.up };
        counter.fetch_add(n as u64, Relaxed);
        if wr.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}

/// Waits out traffic unrelated to the next call, such as runtime events.
async fn quiet(wire: &Wire) {
    let mut last = (wire.up.load(Relaxed), wire.down.load(Relaxed));
    loop {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = (wire.up.load(Relaxed), wire.down.load(Relaxed));
        if now == last {
            return;
        }
        last = now;
    }
}

async fn save(client: &RpcClient, ws: &str, data: &[u8]) -> String {
    client
        .save_attachment(ws.to_string(), "blob.bin".into(), B64.encode(data))
        .await
        .expect("save_attachment")
}

async fn read(client: &RpcClient, path: String) -> Vec<u8> {
    let data = client.read_attachment(path).await.expect("read_attachment");
    B64.decode(data).expect("read_attachment returns base64")
}

fn ratio(wire_bytes: u64) -> f64 {
    (wire_bytes as f64 / PAYLOAD as f64 * 100.0).round() / 100.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachment_wire() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("attachment_wire", &sandbox);
    let server = Server::start(&sandbox).await;
    let project = onboard_and_import(&mut j, &server.client(), &sandbox, &repo).await;
    let view = create_workstream(&mut j, &server.client(), &project, "Wire").await;
    let ws = text(&view, "/item/id");
    let (client, wire) = counted_client(server.socket()).await;

    j.step("a 1 MiB attachment saves and reads back");
    // Incompressible, so no encoding can look cheaper than it is.
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let data: Vec<u8> = (0..PAYLOAD)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();

    quiet(&wire).await;
    let up = wire.up.load(Relaxed);
    let path = save(&client, &ws, &data).await;
    let sent = wire.up.load(Relaxed) - up;

    quiet(&wire).await;
    let down = wire.down.load(Relaxed);
    let back = read(&client, path).await;
    let received = wire.down.load(Relaxed) - down;
    println!(
        "attachment_wire: payload {PAYLOAD} B, save sent {sent} B, read received {received} B"
    );

    j.guard(
        "the bytes read back are the bytes saved",
        back == data,
        json!({ "len": back.len() }),
    );
    j.guard(
        "saving costs at most base64's overhead on the wire",
        ratio(sent) <= MAX_RATIO,
        json!({ "ratio": ratio(sent) }),
    );
    j.guard(
        "reading costs at most base64's overhead on the wire",
        ratio(received) <= MAX_RATIO,
        json!({ "ratio": ratio(received) }),
    );
    j.finish();
}
