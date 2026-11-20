pub mod engines;
pub mod screen;
pub mod text;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::watch;

const RING_CAP_BYTES: usize = 256 * 1024;
const FLUSH_BYTES: usize = 64 * 1024;
/// VS Code's `TerminalDataBufferer` interval: a sustained burst of line-sized writes flushes
/// once per interval, not once per write, so each reader wakes and frames that much less.
const FLUSH_INTERVAL: Duration = Duration::from_millis(5);
/// Flushes less than an interval apart that still go out at once: an echo, and a repaint the
/// pty splits across reads. A timed wait here would cost a timer tick on Windows (~15.6 ms).
const FLUSH_BURST: u32 = 4;
// At most one flush mark per KiB, so the marks stay under 2 KiB however small the flushes.
const MARK_BYTES: u64 = 1024;

/// Chunks the pty reader may run ahead of the coalescer. Each is at most 8 KiB, so
/// the reader can hold ~128 KiB in flight — near VS Code's 100,000-char high
/// watermark. Blocking the reader stops it draining the pty, and the kernel's own
/// buffer then backpressures the shell: the effect of VS Code's
/// `ptyProcess.pause()` without needing an ack protocol, because both ends are
/// threads in one process. Unbounded, a `cat` of a large file outruns the
/// coalescer and the queue grows without limit.
const READ_QUEUE_CHUNKS: usize = 16;
/// How long an exited child's output may take to reach EOF before the exit is signalled anyway.
const EXIT_DRAIN: Duration = Duration::from_millis(500);

pub use hyprlayer_models::terminal::TerminalEvent;

/// A terminal's output as its readers take it: raw bytes at their absolute offset, then the exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalChunk {
    Output { offset: u64, bytes: Vec<u8> },
    Exited { code: Option<i32> },
}

/// Sees every flush, in order.
type OutputTap = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// The one copy of a pty's output: a byte-capped ring every subscriber reads
/// through its own cursor, so scrollback replay and the live stream are the
/// same bytes.
struct OutputRing {
    inner: StdMutex<RingInner>,
    tail_tx: watch::Sender<u64>,
}

struct RingInner {
    bytes: VecDeque<u8>,
    // Absolute offset of bytes[0]. Moves only on trim.
    head: u64,
    // Where a *new* attach starts replaying. Raised by clear_scrollback, which
    // must not move head — an attached reader's cursor may still be below it.
    snapshot_floor: u64,
    // End offsets of retained flushes, ascending, at most one per MARK_BYTES. A trim
    // drops back to one, so head always lands on a flush end — a char boundary.
    flush_ends: VecDeque<u64>,
    exited: Option<Option<i32>>,
    tap: Option<OutputTap>,
}

impl OutputRing {
    fn new() -> Self {
        let (tail_tx, _) = watch::channel(0);
        Self {
            inner: StdMutex::new(RingInner {
                bytes: VecDeque::with_capacity(RING_CAP_BYTES),
                head: 0,
                snapshot_floor: 0,
                flush_ends: VecDeque::new(),
                exited: None,
                tap: None,
            }),
            tail_tx,
        }
    }

    fn push(&self, flush: &[u8]) {
        if flush.is_empty() {
            return;
        }
        let (tail, tap) = {
            let mut inner = self.inner.lock().unwrap();
            inner.make_room(flush.len());
            inner.bytes.extend(flush.iter().copied());
            let tail = inner.tail();
            if inner
                .flush_ends
                .back()
                .is_none_or(|&end| tail - end >= MARK_BYTES)
            {
                inner.flush_ends.push_back(tail);
            }
            (tail, inner.tap.clone())
        };
        // Outside the lock, before the wakeup: one thread pushes, so order holds and a woken reader sees it.
        if let Some(tap) = tap {
            tap(flush);
        }
        self.tail_tx.send_replace(tail);
    }

    /// Feeds `tap` the retained output, then every later flush; `false` when output was already
    /// trimmed. One tap per ring: a second replaces the first.
    fn set_tap(&self, tap: OutputTap) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let (front, back) = inner.bytes.as_slices();
        tap(front);
        tap(back);
        inner.tap = Some(tap);
        inner.head == 0
    }

    fn exit(&self, code: Option<i32>) {
        let tail = {
            let mut inner = self.inner.lock().unwrap();
            inner.exited = Some(code);
            inner.tail()
        };
        self.tail_tx.send_replace(tail);
    }

    // `next_event` without copying: `true` when output moved the cursor, `false` once exited.
    fn advance(&self, cursor: &mut u64) -> Option<bool> {
        let inner = self.inner.lock().unwrap();
        *cursor = (*cursor).max(inner.head);
        let tail = inner.tail();
        if *cursor < tail {
            *cursor = tail;
            return Some(true);
        }
        inner.exited.map(|_| false)
    }

    // One lock for cursor and wakeup, so nothing lands between snapshot and stream.
    // `from` resumes when the ring still holds it; otherwise the replay starts at the floor.
    fn attach(&self, from: Option<u64>) -> (u64, bool, watch::Receiver<u64>) {
        let inner = self.inner.lock().unwrap();
        let resumed = from.filter(|&offset| (inner.head..=inner.tail()).contains(&offset));
        (
            resumed.unwrap_or_else(|| inner.replay_start()),
            resumed.is_some(),
            self.tail_tx.subscribe(),
        )
    }

    fn scrollback_bytes(&self) -> Vec<u8> {
        let inner = self.inner.lock().unwrap();
        let from = (inner.replay_start() - inner.head) as usize;
        inner.bytes.range(from..).copied().collect()
    }

    fn tail_bytes(&self, max: usize) -> Vec<u8> {
        let inner = self.inner.lock().unwrap();
        let from = inner.bytes.len().saturating_sub(max);
        inner.bytes.range(from..).copied().collect()
    }

    fn clear_scrollback(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.snapshot_floor = inner.tail();
    }

    // A cursor below head has lagged; it resumes at head, and the chunk's offset shows the gap.
    fn next_chunk(&self, cursor: &mut u64) -> Option<TerminalChunk> {
        let inner = self.inner.lock().unwrap();
        *cursor = (*cursor).max(inner.head);
        let tail = inner.tail();
        if *cursor == tail {
            return inner.exited.map(|code| TerminalChunk::Exited { code });
        }
        let offset = *cursor;
        *cursor = tail;
        Some(TerminalChunk::Output {
            offset,
            bytes: inner
                .bytes
                .range((offset - inner.head) as usize..)
                .copied()
                .collect(),
        })
    }
}

impl RingInner {
    fn tail(&self) -> u64 {
        self.head + self.bytes.len() as u64
    }

    fn replay_start(&self) -> u64 {
        self.snapshot_floor.max(self.head)
    }

    fn make_room(&mut self, incoming: usize) {
        let mut new_head = self.head;
        while (self.tail() - new_head) as usize + incoming > RING_CAP_BYTES {
            match self.flush_ends.pop_front() {
                Some(end) => new_head = end,
                None => break,
            }
        }
        if new_head > self.head {
            self.bytes.drain(0..(new_head - self.head) as usize);
            self.head = new_head;
            self.snapshot_floor = self.snapshot_floor.max(new_head);
        }
    }
}

/// One subscriber's cursor into a session's output ring.
pub struct RingReader {
    ring: Arc<OutputRing>,
    tail_rx: watch::Receiver<u64>,
    cursor: u64,
    done: bool,
}

impl RingReader {
    fn attach(ring: Arc<OutputRing>, from: Option<u64>) -> (Self, bool) {
        let (cursor, resumed, tail_rx) = ring.attach(from);
        let reader = Self {
            ring,
            tail_rx,
            cursor,
            done: false,
        };
        (reader, resumed)
    }

    /// Everything past the cursor as one `Output`; `Exited` once the ring is
    /// drained and the child is gone; `None` after that. Cancel-safe.
    pub async fn recv(&mut self) -> Option<TerminalChunk> {
        while !self.done {
            if let Some(chunk) = self.ring.next_chunk(&mut self.cursor) {
                self.done = matches!(chunk, TerminalChunk::Exited { .. });
                return Some(chunk);
            }
            if self.tail_rx.changed().await.is_err() {
                break;
            }
        }
        None
    }

    /// Waits for output without copying it; `false` once the child is gone.
    /// A fresh reader's first wake is the retained scrollback.
    pub async fn skip(&mut self) -> bool {
        while !self.done {
            match self.ring.advance(&mut self.cursor) {
                Some(true) => return true,
                Some(false) => self.done = true,
                None => {
                    if self.tail_rx.changed().await.is_err() {
                        break;
                    }
                }
            }
        }
        false
    }
}

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
    ring: Arc<OutputRing>,
    /// `(rows, cols)` as last set.
    size: Arc<StdMutex<(u16, u16)>>,
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
        let ring = Arc::new(OutputRing::new());
        let master = Arc::new(StdMutex::new(Some(pair.master)));

        let session = Arc::new(Self {
            master: master.clone(),
            killer: StdMutex::new(killer),
            writer: StdMutex::new(writer),
            ring: ring.clone(),
            size: Arc::new(StdMutex::new((cfg.rows, cfg.cols))),
            #[cfg(unix)]
            pid,
            workstream_id: cfg.workstream_id,
        });

        let (drained_tx, drained_rx) = std::sync::mpsc::channel::<()>();

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

        let exit_ring = ring.clone();
        std::thread::spawn(move || {
            let mut pending: Vec<u8> = Vec::with_capacity(FLUSH_BYTES);
            let mut last_flush: Option<Instant> = None;
            let mut streak = 0;
            while let Ok(first) = chunk_rx.recv() {
                pending.extend_from_slice(&first);
                let mut disconnected = false;
                let quick = last_flush.is_some_and(|at| at.elapsed() < FLUSH_INTERVAL);
                streak = if quick { streak + 1 } else { 0 };
                // Held here rather than in the ring, so a fast child is backpressured, not trimmed.
                let hold_until = last_flush
                    .filter(|_| streak >= FLUSH_BURST)
                    .map(|at| at + FLUSH_INTERVAL);
                while pending.len() < FLUSH_BYTES {
                    match next_chunk_until(&chunk_rx, hold_until) {
                        Ok(c) => pending.extend_from_slice(&c),
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                // A torn trailing codepoint waits for its continuation, so every
                // flush end — and so every replay start — is a char boundary.
                let whole = pending.len() - torn_utf8_tail(&pending);
                ring.push(&pending[..whole]);
                pending.drain(..whole);
                last_flush = Some(Instant::now());
                if disconnected {
                    break;
                }
            }
            ring.push(&pending);
            let _ = drained_tx.send(());
        });

        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            // Dropped outside the lock and on its own thread: a hung ClosePseudoConsole would hold up `resize`.
            let close_master = || {
                let closing = master.lock().ok().and_then(|mut guard| guard.take());
                std::thread::spawn(move || drop(closing));
            };
            if cfg!(windows) {
                // ConPTY's reader sees EOF only once conhost closes its output, which a hung one never does.
                close_master();
                let _ = drained_rx.recv_timeout(EXIT_DRAIN);
            } else {
                // A Unix reader reaches EOF once every holder of the slave is gone, background jobs included.
                let _ = drained_rx.recv();
                close_master();
            }
            exit_ring.exit(code);
        });

        Ok(session)
    }

    /// A reader positioned at the replay floor: its first `recv()` is the
    /// retained scrollback, and it streams from there.
    pub fn subscribe(&self) -> RingReader {
        RingReader::attach(self.ring.clone(), None).0
    }

    /// A reader from `offset` and `true` while the ring still holds it; otherwise `subscribe`'s and `false`.
    pub fn resume(&self, offset: u64) -> (RingReader, bool) {
        RingReader::attach(self.ring.clone(), Some(offset))
    }

    pub fn scrollback_bytes(&self) -> Vec<u8> {
        self.ring.scrollback_bytes()
    }

    /// The newest `max` output bytes; may start mid-character.
    pub fn tail_bytes(&self, max: usize) -> Vec<u8> {
        self.ring.tail_bytes(max)
    }

    pub fn clear_scrollback(&self) {
        self.ring.clear_scrollback();
    }

    /// Calls `tap` on the coalescer thread with every flush from the first byte, so it never lags
    /// the ring; `false` when the earliest output was already gone.
    pub fn tap_output(&self, tap: impl Fn(&[u8], (u16, u16)) + Send + Sync + 'static) -> bool {
        let size = self.size.clone();
        self.ring
            .set_tap(Arc::new(move |bytes| tap(bytes, *size.lock().unwrap())))
    }

    pub fn write_input(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()
    }

    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        *self.size.lock().unwrap() = (rows, cols);
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

/// A queued chunk, waiting only while a burst is held: an echo never sits in a timed wait.
fn next_chunk_until(
    chunks: &Receiver<Vec<u8>>,
    hold_until: Option<Instant>,
) -> Result<Vec<u8>, RecvTimeoutError> {
    match hold_until.map(|at| at.saturating_duration_since(Instant::now())) {
        Some(wait) if !wait.is_zero() => chunks.recv_timeout(wait),
        _ => chunks.try_recv().map_err(|e| match e {
            TryRecvError::Empty => RecvTimeoutError::Timeout,
            TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
        }),
    }
}

fn to_io(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

// Trailing bytes of a multi-byte codepoint whose continuation has not arrived.
fn torn_utf8_tail(bytes: &[u8]) -> usize {
    (1..=bytes.len().min(3))
        .find(|&k| {
            matches!(
                std::str::from_utf8(&bytes[bytes.len() - k..]),
                Err(e) if e.error_len().is_none()
            )
        })
        .unwrap_or(0)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn first_output(rx: &mut RingReader) -> Vec<u8> {
        match rx.recv().await {
            Some(TerminalChunk::Output { bytes, .. }) => bytes,
            other => panic!("expected an Output first, got {other:?}"),
        }
    }

    async fn drain_until(rx: &mut RingReader, needle: &str) -> String {
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !seen.contains(needle) {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, rx.recv()).await {
                Ok(Some(TerminalChunk::Output { bytes, .. })) => {
                    seen.push_str(&String::from_utf8_lossy(&bytes));
                }
                other => panic!("never saw {needle:?}: {other:?}, seen: {seen:?}"),
            }
        }
        seen
    }

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
            if let Ok(Some(TerminalChunk::Output { bytes, .. })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if seen.contains("MARKER_OK") {
                    break;
                }
            }
        }
        assert!(seen.contains("MARKER_OK"), "got: {seen:?}");
        s.resize(100, 30).expect("resize ok");
        s.kill();
    }

    // `READ_QUEUE_CHUNKS` bounds how far the reader may run ahead of the coalescer,
    // so a burst bigger than that queue blocks the reader. Assert that costs neither
    // bytes nor liveness. Scrollback is the oracle rather than a live subscriber:
    // the output ring is explicitly allowed to drop for a reader that falls
    // behind, so it cannot answer "was anything lost in the queue".
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
        // the coalescer only drained when watched, the tail would never appear.
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
            raw.len() < RING_CAP_BYTES,
            "burst must fit in scrollback for it to be a completeness oracle: {} >= {}",
            raw.len(),
            RING_CAP_BYTES
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
                Ok(Some(TerminalChunk::Output { bytes, .. })) => {
                    seen.push_str(&String::from_utf8_lossy(&bytes));
                }
                Ok(Some(TerminalChunk::Exited { code })) => {
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

        let mut rx = s.subscribe();
        let replay = String::from_utf8_lossy(&first_output(&mut rx).await).into_owned();
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

    // clear_scrollback moves only the replay floor, so a reader attached before
    // the clear keeps every byte and one attached after it sees nothing prior.
    #[tokio::test]
    async fn clear_scrollback_keeps_an_attached_reader_whole_but_not_a_later_one() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-clear-attached".into(),
        })
        .expect("spawn pty");

        let mut early = s.subscribe();
        s.write_input(b"printf 'BEFORE%s\\n' _CLEAR\n").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !String::from_utf8_lossy(&s.scrollback_bytes()).contains("BEFORE_CLEAR") {
            assert!(
                tokio::time::Instant::now() < deadline,
                "BEFORE_CLEAR never reached the ring"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        s.clear_scrollback();
        let mut late = s.subscribe();
        s.write_input(b"printf 'AFTER%s\\n' _CLEAR\n").unwrap();

        let early_saw = drain_until(&mut early, "AFTER_CLEAR").await;
        assert!(
            early_saw.contains("BEFORE_CLEAR"),
            "attached reader lost pre-clear output: {early_saw:?}"
        );
        let late_saw = drain_until(&mut late, "AFTER_CLEAR").await;
        assert!(
            !late_saw.contains("BEFORE_CLEAR"),
            "later attach replayed cleared output: {late_saw:?}"
        );

        s.kill();
    }

    // A trim drops back to a flush end and every flush ends on a char boundary, so
    // a fresh attach after the ring has wrapped decodes cleanly. `cat` re-blocks
    // head's line-per-write() output, so the read chunks here cut lines anywhere.
    #[tokio::test]
    async fn a_trim_lands_on_a_char_boundary() {
        // 3-byte codepoints in a line length no read size divides, so a cut at
        // an arbitrary offset would land mid-codepoint most of the time.
        let line = "\u{6f22}".repeat(41);
        let lines = RING_CAP_BYTES / (line.len() + 2) * 2;
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), format!("yes '{line}' | head -n {lines} | cat")],
            cwd: std::env::temp_dir(),
            cols: 200,
            rows: 50,
            env: vec![],
            workstream_id: "trim".into(),
        })
        .expect("spawn pty");

        let mut live = s.subscribe();
        let mut total = 0usize;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(30), live.recv()).await {
                Ok(Some(TerminalChunk::Output { bytes, .. })) => {
                    total += bytes.len();
                }
                Ok(Some(TerminalChunk::Exited { .. })) => break,
                other => panic!("terminal never exited: {other:?}"),
            }
        }
        assert!(
            total > RING_CAP_BYTES,
            "burst must overflow the ring: {total} <= {RING_CAP_BYTES}"
        );

        let mut fresh = s.subscribe();
        let replay = first_output(&mut fresh).await;
        assert!(replay.len() <= RING_CAP_BYTES, "ring exceeded its cap");
        let text = String::from_utf8(replay).expect("replay starts mid-codepoint");
        assert!(!text.contains('\u{fffd}'));
    }

    #[test]
    fn torn_utf8_tail_holds_back_only_an_incomplete_codepoint() {
        let han = "\u{6f22}".as_bytes();
        assert_eq!(torn_utf8_tail(b""), 0);
        assert_eq!(torn_utf8_tail(b"abc"), 0);
        assert_eq!(torn_utf8_tail(han), 0);
        assert_eq!(torn_utf8_tail(&han[..1]), 1);
        assert_eq!(torn_utf8_tail(&han[..2]), 2);
        assert_eq!(torn_utf8_tail(&[b'a', 0xF0, 0x9F, 0x98]), 3);
        // Invalid or stray bytes are not torn; nothing is coming to complete them.
        assert_eq!(torn_utf8_tail(&[0xE6, b'a']), 0);
        assert_eq!(torn_utf8_tail(&[0x80, 0x80, 0x80]), 0);
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
            if let Ok(Some(TerminalChunk::Output { bytes, .. })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&bytes));
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
