pub mod engines;
pub mod frames;
pub mod screen;
pub mod text;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::{Duration, Instant};

use hyprlayer_models::TerminalTheme;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::watch;

use screen::Screen;

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
#[cfg(unix)]
const HANGUP_GRACE: Duration = Duration::from_millis(100);
const IDLE_COMPRESS: Duration = Duration::from_millis(250);

pub use hyprlayer_models::terminal::TerminalEvent;

/// A terminal's output as its readers take it: raw bytes at their absolute offset, then the exit.
/// A viewer's reader also gets each size the output that follows was written for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalChunk {
    Output { offset: u64, bytes: Vec<u8> },
    Resized { cols: u16, rows: u16 },
    Exited { code: Option<i32> },
}

/// `(rows, cols)`, as `PtySize` orders them.
type Size = (u16, u16);

/// The one copy of a pty's output: a byte-capped ring every subscriber reads
/// through its own cursor, so scrollback replay and the live stream are the
/// same bytes.
struct OutputRing {
    inner: StdMutex<RingInner>,
    tail_tx: watch::Sender<u64>,
    screen: Arc<Screen>,
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
    // The size from each offset on, ascending; the first is the size at head.
    sizes: VecDeque<(u64, Size)>,
    exited: Option<Option<i32>>,
}

impl OutputRing {
    fn new(size: Size, screen: Arc<Screen>) -> Self {
        let (tail_tx, _) = watch::channel(0);
        Self {
            inner: StdMutex::new(RingInner {
                bytes: VecDeque::with_capacity(RING_CAP_BYTES),
                head: 0,
                snapshot_floor: 0,
                flush_ends: VecDeque::new(),
                sizes: VecDeque::from([(0, size)]),
                exited: None,
            }),
            tail_tx,
            screen,
        }
    }

    fn push(&self, flush: &[u8]) {
        if flush.is_empty() {
            return;
        }
        let tail = {
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
            tail
        };
        // Outside the lock, before the wakeup: one thread pushes, so order holds and a woken reader sees it.
        self.screen.feed(flush);
        self.tail_tx.send_replace(tail);
    }

    fn size(&self) -> Size {
        self.inner.lock().unwrap().size()
    }

    /// Marks where output starts being for `size`; `false` when it already is.
    fn set_size(&self, size: Size) -> bool {
        let tail = {
            let mut inner = self.inner.lock().unwrap();
            if inner.size() == size {
                return false;
            }
            let tail = inner.tail();
            // A resize with no output since the last one replaces it.
            if inner.sizes.len() > 1 && inner.sizes.back().is_some_and(|&(at, _)| at == tail) {
                inner.sizes.pop_back();
            }
            if inner.size() != size {
                inner.sizes.push_back((tail, size));
            }
            // Under the ring lock: a quiet pty's screen takes the size before any output does.
            self.screen.resize(size);
            tail
        };
        self.tail_tx.send_replace(tail);
        true
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
    // A chunk cut short ends on a char boundary, since a resume can start at its end.
    // With `shown`, a size change comes before the output written for it, which it bounds.
    fn next_chunk(
        &self,
        cursor: &mut u64,
        max: usize,
        shown: Option<&mut Option<Size>>,
    ) -> Option<TerminalChunk> {
        let inner = self.inner.lock().unwrap();
        *cursor = (*cursor).max(inner.head);
        let mut until = inner.tail();
        if let Some(shown) = shown {
            let (size, changes_at) = inner.size_at(*cursor);
            if *shown != Some(size) {
                *shown = Some(size);
                return Some(TerminalChunk::Resized {
                    cols: size.1,
                    rows: size.0,
                });
            }
            until = changes_at.unwrap_or(until);
        }
        if *cursor == inner.tail() {
            return inner.exited.map(|code| TerminalChunk::Exited { code });
        }
        let offset = *cursor;
        let from = (offset - inner.head) as usize;
        let end = ((until - inner.head) as usize).min(from.saturating_add(max));
        let mut bytes: Vec<u8> = inner.bytes.range(from..end).copied().collect();
        let torn = torn_utf8_tail(&bytes);
        if end < inner.bytes.len() && torn < bytes.len() {
            bytes.truncate(bytes.len() - torn);
        }
        *cursor = offset + bytes.len() as u64;
        Some(TerminalChunk::Output { offset, bytes })
    }
}

impl RingInner {
    fn tail(&self) -> u64 {
        self.head + self.bytes.len() as u64
    }

    fn replay_start(&self) -> u64 {
        self.snapshot_floor.max(self.head)
    }

    fn size(&self) -> Size {
        self.sizes.back().expect("a ring always has a size").1
    }

    // The size output at `offset` was written for, and where the next size starts.
    fn size_at(&self, offset: u64) -> (Size, Option<u64>) {
        let next = self.sizes.partition_point(|&(at, _)| at <= offset);
        (
            self.sizes[next.saturating_sub(1)].1,
            self.sizes.get(next).map(|&(at, _)| at),
        )
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
            while self.sizes.len() > 1 && self.sizes[1].0 <= new_head {
                self.sizes.pop_front();
            }
        }
    }
}

/// A client watching a pty. One that `follows` renders at the pty's size; an older one
/// renders at its own fit, so each fit it sends takes the size. One that `takes` the size
/// gets it only by asking; an older one also gets it by typing or focusing. One that renders
/// `frames` leaves the program's queries to the screen; any other answers them itself.
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub id: u64,
    pub follows: bool,
    pub takes: bool,
    pub frames: bool,
    pub theme: Option<TerminalTheme>,
}

struct Seat {
    viewer: Viewer,
    readers: usize,
    fit: Option<Size>,
}

/// A pty's viewers, the size's owner first, then the rest by how recently they reported a fit:
/// the pty takes the owner's fit, and when the owner leaves the next one owns it.
#[derive(Default)]
struct Viewers(Vec<Seat>);

impl Viewers {
    fn find(&self, id: u64) -> Option<usize> {
        self.0.iter().position(|seat| seat.viewer.id == id)
    }

    fn join(&mut self, viewer: Viewer) {
        match self.find(viewer.id) {
            Some(i) => self.0[i].readers += 1,
            None => self.0.push(Seat {
                viewer,
                readers: 1,
                fit: None,
            }),
        }
    }

    fn leave(&mut self, id: u64) {
        if let Some(i) = self.find(id) {
            self.0[i].readers -= 1;
            if self.0[i].readers == 0 {
                self.0.remove(i);
            }
        }
    }

    /// `false` for a viewer with no reader here.
    fn fit(&mut self, id: u64, size: Size) -> bool {
        let Some(i) = self.find(id) else {
            return false;
        };
        self.0[i].fit = Some(size);
        if !self.0[i].viewer.follows || self.0[0].fit.is_none() {
            self.take(id);
        } else if i > 1 {
            let seat = self.0.remove(i);
            self.0.insert(1, seat);
        }
        true
    }

    /// Typing or focusing, which moves the size only for a viewer that can't ask for it.
    fn claim(&mut self, id: u64) {
        if self.find(id).is_some_and(|i| !self.0[i].viewer.takes) {
            self.take(id);
        }
    }

    fn take(&mut self, id: u64) {
        if let Some(i) = self.find(id) {
            let seat = self.0.remove(i);
            self.0.insert(0, seat);
        }
    }

    fn owner(&self) -> Option<u64> {
        Some(self.0.first()?.viewer.id)
    }

    fn wanted(&self) -> Option<Size> {
        self.0.first()?.fit
    }

    fn set_theme(&mut self, id: u64, theme: TerminalTheme) {
        if let Some(i) = self.find(id) {
            self.0[i].viewer.theme = Some(theme);
        }
    }

    fn screen_answers(&self) -> bool {
        self.0.iter().all(|seat| seat.viewer.frames)
    }

    fn owner_theme(&self) -> Option<TerminalTheme> {
        self.0.first()?.viewer.theme
    }
}

/// Keeps the pty at the size its viewers settle on, marked in the output where it changes.
struct Sizing {
    master: Arc<StdMutex<Option<Box<dyn MasterPty + Send>>>>,
    ring: Arc<OutputRing>,
    viewers: StdMutex<Viewers>,
}

impl Sizing {
    fn update(&self, change: impl FnOnce(&mut Viewers)) {
        let mut viewers = self.viewers.lock().unwrap_or_else(PoisonError::into_inner);
        let owner = viewers.owner();
        change(&mut viewers);
        self.settle(&viewers);
        if viewers.owner() != owner {
            // A new owner at the same size changes no output, so its readers are woken to tell their clients.
            self.ring.tail_tx.send_modify(|_| {});
        }
    }

    fn settle(&self, viewers: &Viewers) {
        if let Some(size) = viewers.wanted() {
            self.set(size);
        }
        self.ring
            .screen
            .follow(viewers.screen_answers(), viewers.owner_theme());
    }

    // Marked before the resize: output read after it may already answer the SIGWINCH.
    fn set(&self, (rows, cols): Size) {
        if !self.ring.set_size((rows, cols)) {
            return;
        }
        if let Some(master) = self.master.lock().unwrap().as_ref() {
            let size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            if let Err(e) = master.resize(size) {
                tracing::debug!("pty resize to {cols}x{rows} failed: {e}");
            }
        }
    }

    fn fit(&self, viewer: Option<u64>, size: Size) {
        self.update(|viewers| {
            let seated = viewer.is_some_and(|id| viewers.fit(id, size));
            if !seated && viewers.0.is_empty() {
                // No viewer to weigh it against, as for an in-process caller: the fit stands.
                self.set(size);
            }
        });
    }
}

struct ReaderSeat {
    id: u64,
    follows: bool,
    shown: Option<Size>,
}

/// One subscriber's cursor into a session's output ring.
pub struct RingReader {
    sizing: Arc<Sizing>,
    tail_rx: watch::Receiver<u64>,
    cursor: u64,
    done: bool,
    seat: Option<ReaderSeat>,
}

impl Drop for RingReader {
    fn drop(&mut self) {
        if let Some(seat) = &self.seat {
            self.sizing.update(|viewers| viewers.leave(seat.id));
        }
    }
}

impl RingReader {
    fn attach(sizing: Arc<Sizing>, from: Option<u64>) -> (Self, bool) {
        let (cursor, resumed, tail_rx) = sizing.ring.attach(from);
        let reader = Self {
            sizing,
            tail_rx,
            cursor,
            done: false,
            seat: None,
        };
        (reader, resumed)
    }

    /// `viewer`'s reader: it counts toward the pty's size until dropped, and a viewer that
    /// follows also gets each size in line with the output.
    pub fn view(mut self, viewer: Viewer) -> Self {
        if self.seat.is_none() {
            self.sizing.update(|viewers| viewers.join(viewer));
            self.seat = Some(ReaderSeat {
                id: viewer.id,
                follows: viewer.follows,
                shown: None,
            });
        }
        self
    }

    /// Everything past the cursor as one `Output`; `Exited` once the ring is
    /// drained and the child is gone; `None` after that. Cancel-safe.
    pub async fn recv(&mut self) -> Option<TerminalChunk> {
        self.recv_at_most(usize::MAX).await
    }

    /// `recv`, with at most `max` bytes in an `Output`.
    pub async fn recv_at_most(&mut self, max: usize) -> Option<TerminalChunk> {
        while !self.done {
            let shown = self
                .seat
                .as_mut()
                .filter(|seat| seat.follows)
                .map(|seat| &mut seat.shown);
            if let Some(chunk) = self.sizing.ring.next_chunk(&mut self.cursor, max, shown) {
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
            match self.sizing.ring.advance(&mut self.cursor) {
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

    pub fn screen(&self) -> Arc<Screen> {
        self.sizing.ring.screen.clone()
    }

    /// Waits for output, a size or the exit; `false` once nothing more can come.
    pub async fn changed(&mut self) -> bool {
        self.tail_rx.changed().await.is_ok()
    }

    /// The child's exit code, once all of its output is on the screen.
    pub fn exited(&self) -> Option<Option<i32>> {
        self.sizing.ring.inner.lock().unwrap().exited
    }

    pub fn owns_size(&self) -> bool {
        let viewers = self
            .sizing
            .viewers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let owner = viewers.owner();
        self.seat
            .as_ref()
            .is_some_and(|seat| owner == Some(seat.id))
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

type PtyWriter = Arc<StdMutex<Box<dyn Write + Send>>>;

pub struct PtySession {
    killer: StdMutex<Box<dyn ChildKiller + Send + Sync>>,
    writer: PtyWriter,
    sizing: Arc<Sizing>,
    #[cfg(unix)]
    pid: Option<u32>,
    killed: Arc<AtomicBool>,
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
        let writer: PtyWriter = Arc::new(StdMutex::new(pair.master.take_writer().map_err(to_io)?));
        let (replies, answered) = std::sync::mpsc::channel();
        let size = (cfg.rows, cfg.cols);
        let screen = Screen::new(size, screen::spawn_theme(&cfg.env), replies).map_err(to_io)?;
        let ring = Arc::new(OutputRing::new(size, Arc::new(screen)));
        let master = Arc::new(StdMutex::new(Some(pair.master)));

        let reply_writer = writer.clone();
        // Its own thread: a child that doesn't read its input must not stop its output being read.
        std::thread::spawn(move || {
            for reply in answered {
                let mut w = reply_writer.lock().unwrap_or_else(PoisonError::into_inner);
                if w.write_all(&reply).and_then(|()| w.flush()).is_err() {
                    break;
                }
            }
        });

        let killed = Arc::new(AtomicBool::new(false));
        let session = Arc::new(Self {
            killer: StdMutex::new(killer),
            writer,
            sizing: Arc::new(Sizing {
                master: master.clone(),
                ring: ring.clone(),
                viewers: StdMutex::default(),
            }),
            #[cfg(unix)]
            pid,
            killed: killed.clone(),
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
            loop {
                let first = match chunk_rx.recv_timeout(IDLE_COMPRESS) {
                    Ok(chunk) => chunk,
                    Err(RecvTimeoutError::Timeout) => {
                        ring.screen.compress();
                        match chunk_rx.recv() {
                            Ok(chunk) => chunk,
                            Err(_) => break,
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
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
            } else if killed.load(Ordering::SeqCst) {
                // A job that ignored the hangup still holds the slave, so the reader may never see EOF.
                let _ = drained_rx.recv_timeout(EXIT_DRAIN);
                close_master();
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
        RingReader::attach(self.sizing.clone(), None).0
    }

    /// A reader from `offset` and `true` while the ring still holds it; otherwise `subscribe`'s and `false`.
    pub fn resume(&self, offset: u64) -> (RingReader, bool) {
        RingReader::attach(self.sizing.clone(), Some(offset))
    }

    pub fn scrollback_bytes(&self) -> Vec<u8> {
        self.sizing.ring.scrollback_bytes()
    }

    /// The newest `max` output bytes; may start mid-character.
    pub fn tail_bytes(&self, max: usize) -> Vec<u8> {
        self.sizing.ring.tail_bytes(max)
    }

    pub fn clear_scrollback(&self) {
        self.sizing.ring.clear_scrollback();
    }

    pub fn screen(&self) -> Arc<Screen> {
        self.sizing.ring.screen.clone()
    }

    pub fn write_input(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()
    }

    /// `(rows, cols)`.
    pub fn size(&self) -> (u16, u16) {
        self.sizing.ring.size()
    }

    /// `viewer`'s fit, which sizes the pty once that viewer is the most recently active.
    pub fn fit(&self, viewer: Option<u64>, cols: u16, rows: u16) {
        if cols > 0 && rows > 0 {
            self.sizing.fit(viewer, (rows, cols));
        }
    }

    /// `viewer` typed or focused, so the pty takes its fit if it is a viewer that can't ask.
    pub fn claim(&self, viewer: u64) {
        self.sizing.update(|viewers| viewers.claim(viewer));
    }

    pub fn take_size(&self, viewer: u64) {
        self.sizing.update(|viewers| viewers.take(viewer));
    }

    pub fn set_theme(&self, viewer: u64, theme: TerminalTheme) {
        self.sizing
            .update(|viewers| viewers.set_theme(viewer, theme));
    }

    pub fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
            if pgid > 0 {
                // An interactive shell runs each background job in its own group and forwards a
                // hangup to them, but a SIGKILLed shell forwards nothing: hang up first.
                unsafe { libc::killpg(pgid, libc::SIGHUP) };
                std::thread::sleep(HANGUP_GRACE);
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
    use hyprlayer_models::Rgb;

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
        s.fit(None, 100, 30);
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

    /// Sets mode 2031, prints each chunk it reads as `GOT <hex>`, and after a lone `q` asks `$QUERY`.
    const ASKS: &str = r#"stty raw -echo min 0 time 3; printf '\033[?2031hREADY\r\n'; while :; do c=$(dd bs=256 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); [ -z "$c" ] && continue; printf 'GOT %s\r\n' "$c"; [ "$c" = 71 ] && printf "$QUERY\r\n"; done"#;
    const QUIET: Duration = Duration::from_millis(1200);

    fn asking(query: &str, env: &[(&str, &str)]) -> Arc<PtySession> {
        let mut vars = vec![("QUERY".to_string(), query.to_string())];
        vars.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), ASKS.into()],
            cwd: std::env::temp_dir(),
            cols: 120,
            rows: 40,
            env: vars,
            workstream_id: "asks".into(),
        })
        .expect("spawn pty")
    }

    fn heard(s: &PtySession) -> Vec<String> {
        String::from_utf8_lossy(&s.scrollback_bytes())
            .split("\r\n")
            .filter_map(|line| line.strip_prefix("GOT ").map(str::to_string))
            .collect()
    }

    async fn settled(s: &PtySession, what: impl Fn(&PtySession) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !what(s) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the pty never settled"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn answers(s: &PtySession) -> String {
        settled(s, |s| {
            String::from_utf8_lossy(&s.scrollback_bytes()).contains("READY")
        })
        .await;
        let before = heard(s).len();
        s.write_input(b"q").unwrap();
        settled(s, |s| heard(s).get(before).is_some_and(|got| got == "71")).await;
        tokio::time::sleep(QUIET).await;
        let hex: String = heard(s)[before + 1..].concat();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn theme(background: Rgb, dark: bool) -> TerminalTheme {
        TerminalTheme {
            foreground: Rgb(0x33, 0x33, 0x33),
            background,
            cursor: Rgb(0xd9, 0x77, 0x57),
            ansi: [Rgb(0x10, 0x20, 0x30); 16],
            dark,
        }
    }

    fn viewer(id: u64, frames: bool, theme: Option<TerminalTheme>) -> Viewer {
        Viewer {
            id,
            follows: true,
            takes: true,
            frames,
            theme,
        }
    }

    #[tokio::test]
    async fn a_pty_nobody_watches_answers_its_queries_in_the_spawn_theme() {
        let s = asking(r"\033[c\033[?996n\033]11;?\033\\", &[("COLORFGBG", "0;15")]);
        assert_eq!(
            answers(&s).await,
            "\x1b[?62;22c\x1b[?997;2n\x1b]11;rgb:f7f7/f7f7/f7f7\x1b\\"
        );
        s.kill();
    }

    #[tokio::test]
    async fn a_raw_viewer_answers_for_itself_and_a_frame_viewer_leaves_it_to_the_screen() {
        let s = asking(r"\033[c", &[]);
        let raw = s.subscribe().view(viewer(1, false, None));
        assert_eq!(
            answers(&s).await,
            "",
            "the screen answered over a raw viewer"
        );
        drop(raw);
        assert_eq!(answers(&s).await, "\x1b[?62;22c");
        let _frames = s.subscribe().view(viewer(2, true, None));
        assert_eq!(answers(&s).await, "\x1b[?62;22c");
        s.kill();
    }

    #[tokio::test]
    async fn the_size_owner_s_theme_answers_and_a_flip_in_darkness_is_reported() {
        let dark = theme(Rgb(0x10, 0x10, 0x10), true);
        let light = theme(Rgb(0xfa, 0xfa, 0xfa), false);
        let s = asking(r"\033]11;?\033\\\033]4;1;?\033\\", &[]);
        let _a = s.subscribe().view(viewer(1, true, Some(dark)));
        let _b = s.subscribe().view(viewer(2, true, Some(light)));
        assert_eq!(
            answers(&s).await,
            "\x1b]11;rgb:1010/1010/1010\x1b\\\x1b]4;1;rgb:1010/2020/3030\x1b\\"
        );

        let before = heard(&s).len();
        s.take_size(2);
        settled(&s, |s| {
            heard(s)[before..].concat().contains("1b5b3f3939373b326e")
        })
        .await;
        assert!(answers(&s)
            .await
            .starts_with("\x1b]11;rgb:fafa/fafa/fafa\x1b\\"));

        let before = heard(&s).len();
        s.set_theme(2, dark);
        settled(&s, |s| {
            heard(s)[before..].concat().contains("1b5b3f3939373b316e")
        })
        .await;
        s.kill();
    }

    #[tokio::test]
    async fn the_screen_takes_a_resize_before_any_output_follows_it() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "printf READY; sleep 30".into()],
            cwd: std::env::temp_dir(),
            cols: 120,
            rows: 40,
            env: vec![],
            workstream_id: "quiet".into(),
        })
        .expect("spawn pty");
        let screen = s.screen();
        settled(&s, |_| {
            screen
                .rows()
                .is_some_and(|rows| rows.first().is_some_and(|r| r == "READY"))
        })
        .await;
        assert_eq!(screen.rows().map(|rows| rows.len()), Some(40));
        s.fit(None, 80, 24);
        let rows = screen.rows().expect("the screen reads");
        assert_eq!((rows.len(), rows[0].as_str()), (24, "READY"));
        s.kill();
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

    #[tokio::test]
    async fn kill_hangs_up_an_interactive_shells_background_jobs() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/bash"),
            args: vec!["--norc".into(), "-i".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-jobs".into(),
        })
        .expect("spawn pty");
        let mut rx = s.subscribe();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        s.write_input(b"sleep 600 & echo PID:$!\n").unwrap();
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let job: i32 = loop {
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
                    .filter(|_| {
                        seen.rsplit_once("PID:")
                            .and_then(|(_, after)| after.split_once('\n'))
                            .is_some_and(|(_, prompt)| prompt.contains("$ "))
                    })
                {
                    break pid;
                }
            }
        };
        // Linux bash defers a hangup that lands before its prompt is reading input.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        s.kill();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline
            && (unsafe { libc::kill(job, 0) } == 0 || rx.exited().is_none())
        {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let alive = unsafe { libc::kill(job, 0) } == 0;
        unsafe { libc::kill(job, libc::SIGKILL) };
        assert!(!alive, "background job {job} survived kill()");
        assert!(
            rx.exited().is_some(),
            "the killed terminal never reported its exit"
        );
    }
}
