use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyprlayer_proto::Features;
use hyprlayer_server::client::RpcClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const MIB: u64 = 1024 * 1024;
/// What a buffering link (an SSH channel, a bufferbloated cell link) holds downstream.
pub const DEEP_LINK_BYTES: u64 = 4 * MIB;
/// What ssh's channel window and the socket buffers under it hold upstream.
pub const DEEP_UPLINK_BYTES: u64 = 2 * MIB;

/// One connection's two directions, as a proxy that can stall, throttle or buffer them.
#[derive(Default)]
pub struct Link {
    pub down_stalled: AtomicBool,
    pub down_bps: AtomicU64,
    pub up_bps: AtomicU64,
    /// Read from the server, not yet delivered to the client.
    pub buffered: AtomicU64,
    /// Read from the client, not yet delivered to the server.
    pub up_buffered: AtomicU64,
    /// When each runtime event reached the client.
    pub events: Arc<Mutex<Vec<Instant>>>,
    pub delivered: AtomicU64,
}

/// `deep`: the link takes up to `DEEP_LINK_BYTES` from the server at once and
/// delivers it at `down_bps`; otherwise it reads only as fast as it delivers.
/// `deep_up`: the same toward the server, up to `DEEP_UPLINK_BYTES`, as ssh's channel
/// window and socket buffers hold. `one_way` delays each direction.
pub async fn far_phone(
    socket: &Path,
    deep: bool,
    one_way: Duration,
    deep_up: bool,
) -> (Arc<RpcClient>, Arc<Link>) {
    far_phone_announcing(socket, deep, one_way, deep_up, None).await
}

pub async fn far_phone_announcing(
    socket: &Path,
    deep: bool,
    one_way: Duration,
    deep_up: bool,
    features: Option<Features>,
) -> (Arc<RpcClient>, Arc<Link>) {
    let unix = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the sandbox socket");
    let (mut srv_rd, mut srv_wr) = unix.into_split();
    let (client_end, proxy_end) = tokio::io::duplex(64 * 1024);
    let (mut cli_rd, mut cli_wr) = tokio::io::split(proxy_end);
    let link = Arc::new(Link::default());
    // A shallow uplink is paced in step with its reads, so it pushes back on the client.
    let (up_tx, mut up_rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let up = link.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            while deep_up && up.up_buffered.load(Relaxed) > DEEP_UPLINK_BYTES {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let n = match cli_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            up.up_buffered.fetch_add(n as u64, Relaxed);
            if up_tx
                .send((Instant::now() + one_way, buf[..n].to_vec()))
                .is_err()
            {
                break;
            }
            if !deep_up {
                pace(n, up.up_bps.load(Relaxed)).await;
            }
        }
    });
    let up_writer = link.clone();
    tokio::spawn(async move {
        let mut free_at = Instant::now();
        while let Some((due, chunk)) = up_rx.recv().await {
            tokio::time::sleep_until(due.into()).await;
            if srv_wr.write_all(&chunk).await.is_err() {
                break;
            }
            up_writer.up_buffered.fetch_sub(chunk.len() as u64, Relaxed);
            let bps = up_writer.up_bps.load(Relaxed);
            if deep_up && bps > 0 {
                free_at = free_at
                    .max(Instant::now() - Duration::from_millis(5))
                    .max(due);
                free_at += Duration::from_secs_f64(chunk.len() as f64 / bps as f64);
                tokio::time::sleep_until(free_at.into()).await;
            }
        }
    });
    let (tx, mut rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let filler = link.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            while filler.down_stalled.load(Relaxed)
                || (deep && filler.buffered.load(Relaxed) > DEEP_LINK_BYTES)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let n = match srv_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            filler.buffered.fetch_add(n as u64, Relaxed);
            if tx
                .send((Instant::now() + one_way, buf[..n].to_vec()))
                .is_err()
            {
                break;
            }
            if !deep {
                while filler.buffered.load(Relaxed) > 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }
    });
    let down = link.clone();
    tokio::spawn(async move {
        // Paced against a running deadline: a timer's millisecond overshoot is made up, not lost.
        let mut free_at = Instant::now();
        while let Some((due, chunk)) = rx.recv().await {
            tokio::time::sleep_until(due.into()).await;
            while down.down_stalled.load(Relaxed) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if cli_wr.write_all(&chunk).await.is_err() {
                return;
            }
            down.buffered.fetch_sub(chunk.len() as u64, Relaxed);
            down.delivered.fetch_add(chunk.len() as u64, Relaxed);
            let bps = down.down_bps.load(Relaxed);
            let now = Instant::now();
            free_at = free_at.max(now - Duration::from_millis(5)).max(due);
            if bps > 0 {
                free_at += Duration::from_secs_f64(chunk.len() as f64 / bps as f64);
                tokio::time::sleep_until(free_at.into()).await;
            }
        }
    });
    let client = match features {
        Some(features) => RpcClient::connect_announcing(client_end, features).await,
        None => RpcClient::connect(client_end).await,
    }
    .expect("handshake");
    drain_events(&client, link.events.clone());
    (client, link)
}

async fn pace(bytes: usize, bps: u64) {
    if bps > 0 {
        tokio::time::sleep(Duration::from_secs_f64(bytes as f64 / bps as f64)).await;
    }
}

/// Notes when each runtime event arrived.
fn drain_events(client: &RpcClient, arrivals: Arc<Mutex<Vec<Instant>>>) {
    let mut events = client.take_events().expect("runtime events");
    tokio::spawn(async move {
        while events.recv().await.is_some() {
            arrivals.lock().unwrap().push(Instant::now());
        }
    });
}
