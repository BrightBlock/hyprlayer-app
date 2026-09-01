use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use base64::Engine as _;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

const SCROLLBACK_CAP: usize = 256 * 1024;
const FLUSH_BYTES: usize = 64 * 1024;

/// Chunks the pty reader may run ahead of the encoder. Each is at most 8 KiB, so
/// the reader can hold ~128 KiB in flight — near VS Code's 100,000-char high
/// watermark. Blocking the reader stops it draining the pty, and the kernel's own
/// buffer then backpressures the shell: the effect of VS Code's
/// `ptyProcess.pause()` without needing an ack protocol, because both ends are
/// threads in one process. Unbounded, a `cat` of a large file outruns base64
/// encoding and the queue grows without limit.
const READ_QUEUE_CHUNKS: usize = 16;

/// Output events retained for subscribers that fall behind. The ring's cost is
/// `capacity x event size`, and an event is one encoder flush: a few KiB when the
/// pty trickles, up to `FLUSH_BYTES` of base64 (~87 KiB) when a batch fills. So the
/// ceiling is high (~178 MiB all-worst-case) but the common case is far smaller.
///
/// Left at 2048 deliberately. Shrinking it to 256 — matching the per-attachment
/// queue downstream — was tried and reverted: a plain `seq 1 50000` burst emits
/// well over 256 flushes, so a prompt subscriber took a `Lagged` gap and lost
/// output. Bounding this properly needs byte-based accounting or an ack protocol,
/// not a smaller constant. `a_burst_larger_than_the_reader_queue_arrives_whole_and_in_order`
/// fails if someone shrinks it again without that.
const OUTPUT_RING_EVENTS: usize = 2048;
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

pub use hyprlayer_models::terminal::TerminalEvent;

pub struct SpawnTerminal {
    pub shell: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
    pub workstream_id: String,
}

pub struct PtySession {
    master: Arc<StdMutex<Option<Box<dyn MasterPty + Send>>>>,
    killer: StdMutex<Box<dyn ChildKiller + Send + Sync>>,
    writer: StdMutex<Box<dyn Write + Send>>,
    tx: broadcast::Sender<TerminalEvent>,
    scrollback: Arc<StdMutex<VecDeque<u8>>>,
    #[cfg(unix)]
    pid: Option<u32>,
    pub workstream_id: String,
}

impl PtySession {
    pub fn spawn(cfg: SpawnTerminal) -> std::io::Result<Arc<Self>> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: cfg.rows,
                cols: cfg.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(to_io)?;

        let mut cmd = CommandBuilder::new(&cfg.shell);
        for arg in &cfg.args {
            cmd.arg(arg);
        }
        cmd.cwd(&cfg.cwd);
        cmd.env("TERM", "xterm-256color");
        for (key, value) in crate::process::appimage::child_env_overrides() {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            }
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(to_io)?;
        drop(pair.slave);
        let killer = child.clone_killer();
        #[cfg(unix)]
        let pid = child.process_id();

        let reader = pair.master.try_clone_reader().map_err(to_io)?;
        let writer = pair.master.take_writer().map_err(to_io)?;
        let (tx, _rx) = broadcast::channel(OUTPUT_RING_EVENTS);
        let scrollback = Arc::new(StdMutex::new(VecDeque::with_capacity(SCROLLBACK_CAP)));
        let master = Arc::new(StdMutex::new(Some(pair.master)));

        let session = Arc::new(Self {
            master: master.clone(),
            killer: StdMutex::new(killer),
            writer: StdMutex::new(writer),
            tx: tx.clone(),
            scrollback: scrollback.clone(),
            #[cfg(unix)]
            pid,
            workstream_id: cfg.workstream_id,
        });

        let (code_tx, code_rx) = std::sync::mpsc::channel::<Option<i32>>();

        let (chunk_tx, chunk_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(READ_QUEUE_CHUNKS);

        let mut reader = reader;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) if chunk_tx.send(buf[..n].to_vec()).is_err() => break,
                    Ok(_) => {}
                }
            }
        });

        let reader_tx = tx.clone();
        std::thread::spawn(move || {
            let mut pending: Vec<u8> = Vec::with_capacity(FLUSH_BYTES);
            let flush = |pending: &mut Vec<u8>| {
                if pending.is_empty() {
                    return;
                }
                let encoded = B64.encode(&pending);
                let mut sb = scrollback.lock().unwrap();
                sb.extend(pending.iter().copied());
                let overflow = sb.len().saturating_sub(SCROLLBACK_CAP);
                if overflow > 0 {
                    sb.drain(0..overflow);
                }
                let _ = reader_tx.send(TerminalEvent::Output { data: encoded });
                drop(sb);
                pending.clear();
            };
            while let Ok(first) = chunk_rx.recv() {
                pending.extend_from_slice(&first);
                let mut disconnected = false;
                while pending.len() < FLUSH_BYTES {
                    match chunk_rx.try_recv() {
                        Ok(c) => pending.extend_from_slice(&c),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                flush(&mut pending);
                if disconnected {
                    break;
                }
            }
            let code = code_rx.recv().unwrap_or(None);
            let _ = reader_tx.send(TerminalEvent::Exited { code });
        });

        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            let _ = code_tx.send(code);
            if let Ok(mut guard) = master.lock() {
                let _ = guard.take();
            }
        });

        Ok(session)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.tx.subscribe()
    }

    pub fn subscribe_with_scrollback(&self) -> (broadcast::Receiver<TerminalEvent>, String) {
        let sb = self.scrollback.lock().unwrap();
        let rx = self.tx.subscribe();
        let snapshot = B64.encode(sb.iter().copied().collect::<Vec<u8>>());
        (rx, snapshot)
    }

    pub fn scrollback_bytes(&self) -> Vec<u8> {
        self.scrollback.lock().unwrap().iter().copied().collect()
    }

    pub fn clear_scrollback(&self) {
        self.scrollback.lock().unwrap().clear();
    }

    pub fn write_input(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        match self.master.lock().unwrap().as_ref() {
            Some(master) => master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(to_io),
            None => Ok(()),
        }
    }

    pub fn kill(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
            if pgid > 0 {
                unsafe { libc::killpg(pgid, libc::SIGKILL) };
            }
        }
        let _ = self.killer.lock().unwrap().kill();
    }
}

fn to_io(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pty_echo_roundtrip() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test".into(),
        })
        .expect("spawn pty");

        let mut rx = s.subscribe();
        s.write_input(b"printf MARKER_OK\n").unwrap();

        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Ok(TerminalEvent::Output { data })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
                if seen.contains("MARKER_OK") {
                    break;
                }
            }
        }
        assert!(seen.contains("MARKER_OK"), "got: {seen:?}");
        s.resize(100, 30).expect("resize ok");
        s.kill();
    }

    // `READ_QUEUE_CHUNKS` bounds how far the reader may run ahead of the encoder,
    // so a burst bigger than that queue blocks the reader. Assert that costs neither
    // bytes nor liveness. Scrollback is the oracle rather than a live subscriber:
    // the broadcast ring is explicitly allowed to drop for a receiver that falls
    // behind (`Lagged`), so it cannot answer "was anything lost in the queue".
    #[tokio::test]
    async fn a_burst_larger_than_the_reader_queue_drains_whole_and_in_order() {
        const LAST: usize = 30_000;

        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), format!("seq 1 {LAST}")],
            cwd: std::env::temp_dir(),
            cols: 200,
            rows: 50,
            env: vec![],
            workstream_id: "burst".into(),
        })
        .expect("spawn pty");

        // Nobody subscribes: if a full reader queue could wedge the pipeline, or if
        // the encoder only drained when watched, the tail would never appear.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut raw = Vec::new();
        while tokio::time::Instant::now() < deadline {
            raw = s.scrollback_bytes();
            if String::from_utf8_lossy(&raw).contains(&format!("\r\n{LAST}\r\n")) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        assert!(
            raw.len() < SCROLLBACK_CAP,
            "burst must fit in scrollback for it to be a completeness oracle: {} >= {}",
            raw.len(),
            SCROLLBACK_CAP
        );
        assert!(
            raw.len() > READ_QUEUE_CHUNKS * 8192,
            "burst must exceed the in-flight bound to exercise backpressure: {} <= {}",
            raw.len(),
            READ_QUEUE_CHUNKS * 8192
        );

        let numbers: Vec<usize> = String::from_utf8_lossy(&raw)
            .split(['\r', '\n'])
            .filter_map(|line| line.parse::<usize>().ok())
            .collect();
        assert_eq!(
            numbers,
            (1..=LAST).collect::<Vec<_>>(),
            "output was dropped, reordered or corrupted (saw {} numbers)",
            numbers.len()
        );

        s.kill();
    }

    #[tokio::test]
    async fn spawn_with_args_runs_and_exits() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/echo"),
            args: vec!["ARGS_OK".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-args".into(),
        })
        .expect("spawn pty");

        let mut rx = s.subscribe();
        let mut seen = String::new();
        let mut exited = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(TerminalEvent::Output { data })) => {
                    seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
                }
                Ok(Ok(TerminalEvent::Exited { code })) => {
                    assert_eq!(code, Some(0), "echo should exit 0");
                    exited = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(seen.contains("ARGS_OK"), "got: {seen:?}");
        assert!(exited, "expected Exited event");
    }

    #[tokio::test]
    async fn open_path_delivers_live_output() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test".into(),
        })
        .expect("spawn pty");

        let (mut rx, snapshot) = s.subscribe_with_scrollback();
        let mut seen = String::from_utf8_lossy(&B64.decode(snapshot).unwrap()).into_owned();
        s.write_input(b"printf MARKER_OK\n").unwrap();

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !seen.contains("MARKER_OK") && tokio::time::Instant::now() < deadline {
            if let Ok(Ok(TerminalEvent::Output { data })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
            }
        }
        assert!(seen.contains("MARKER_OK"), "got: {seen:?}");
        s.kill();
    }

    #[tokio::test]
    async fn clear_scrollback_drops_prior_output_but_keeps_streaming() {
        async fn wait_for(s: &PtySession, needle: &str) {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                let seen = String::from_utf8_lossy(&s.scrollback_bytes()).into_owned();
                if seen.contains(needle) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!("timed out waiting for {needle:?} in scrollback");
        }

        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-clear".into(),
        })
        .expect("spawn pty");

        s.write_input(b"printf 'BEFORE%s\\n' _CLEAR\n").unwrap();
        wait_for(&s, "BEFORE_CLEAR").await;

        s.clear_scrollback();

        s.write_input(b"printf 'AFTER%s\\n' _CLEAR\n").unwrap();
        wait_for(&s, "AFTER_CLEAR").await;

        let snapshot = String::from_utf8_lossy(&s.scrollback_bytes()).into_owned();
        assert!(
            !snapshot.contains("BEFORE_CLEAR"),
            "exit snapshot resurrected cleared output: {snapshot:?}"
        );

        let (_rx, replay) = s.subscribe_with_scrollback();
        let replay = String::from_utf8_lossy(&B64.decode(replay).unwrap()).into_owned();
        assert!(
            !replay.contains("BEFORE_CLEAR"),
            "re-attach replayed cleared output: {replay:?}"
        );
        assert!(
            replay.contains("AFTER_CLEAR"),
            "post-clear output must still stream: {replay:?}"
        );

        s.kill();
    }

    #[tokio::test]
    async fn kill_reaps_child_process_group() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "sleep 60 & echo PID:$!; sleep 60".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-group".into(),
        })
        .expect("spawn pty");

        let mut rx = s.subscribe();
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let child_pid: i32 = loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "never saw pid: {seen:?}"
            );
            if let Ok(Ok(TerminalEvent::Output { data })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
                if let Some(pid) = seen
                    .lines()
                    .find_map(|l| l.strip_prefix("PID:"))
                    .and_then(|p| p.trim().parse::<i32>().ok())
                {
                    break pid;
                }
            }
        };

        assert_eq!(
            unsafe { libc::kill(child_pid, 0) },
            0,
            "backgrounded child should be alive before kill()"
        );
        s.kill();

        let mut gone = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if unsafe { libc::kill(child_pid, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(gone, "backgrounded child {child_pid} survived kill()");
    }
}
