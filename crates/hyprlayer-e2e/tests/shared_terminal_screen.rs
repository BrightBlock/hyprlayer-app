//! Two desktops, A at 120x40 dark and B at 80x24 light, render one shared terminal as the app does:
//! they announce `screen_frames` and write the server's frames into xterm 5.5 (`xterm/render.mjs`,
//! which needs `node` and the repository's `pnpm install`). Every case compares their screens cell
//! by cell. A raw tap, which renders and answers nothing, feeds a libghostty model the bytes the
//! program wrote; frame desktops C and D must show what that model holds, cells and history.
//! Each frame desktop also feeds a `FrameScreen` (`hyprlayer-client`), whose cells and history
//! must equal the model's exactly.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ghostty_vt::{CellColor, CellStyle, CellWide, RowAt, Terminal};
use hyprlayer_client::terminal::{FrameScreen, ScreenCell};
use hyprlayer_core::models::{Rgb, TerminalTheme};
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_e2e::agent_stub::{install, SCREEN_TUI};
use hyprlayer_e2e::link::{far_phone, far_phone_announcing};
use hyprlayer_e2e::{eventually, json, keystrokes, Journey, Sandbox, Server, Value, RAW_DESKTOP};
use hyprlayer_proto::Features;
use hyprlayer_server::client::{RpcClient, TerminalStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

const WAIT: Duration = Duration::from_secs(30);
const STALL: Duration = Duration::from_secs(5);
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../hyprlayer-core/tests/fixtures/claude_tui/effort-2.1.330-120x40.bin"
);

/// A light desktop's OSC 11 answer, typed as input to see it reach the program.
const LIGHT_REPLY: &str = "\x1b]11;rgb:ffff/ffff/ffff\x1b\\";
const A_THEME: TerminalTheme = TerminalTheme {
    foreground: Rgb(0xff, 0xff, 0xff),
    background: Rgb(0, 0, 0),
    cursor: Rgb(0xff, 0xff, 0xff),
    ansi: [Rgb(0x80, 0x80, 0x80); 16],
    dark: true,
};
const LIGHT_THEME: TerminalTheme = TerminalTheme {
    foreground: Rgb(0, 0, 0),
    background: Rgb(0xff, 0xff, 0xff),
    cursor: Rgb(0, 0, 0),
    ansi: [Rgb(0x80, 0x80, 0x80); 16],
    dark: false,
};

/// The newest `(paint N)` the stub drew in `buf` that ends at or past `from`.
fn painted_in(buf: &[u8], from: usize) -> Option<u64> {
    find(buf, b"(paint ")
        .filter_map(|at| {
            let digits = &buf[at + 7..];
            let len = digits.iter().take_while(|c| c.is_ascii_digit()).count();
            if digits.get(len) != Some(&b')') || at + 7 + len < from {
                return None;
            }
            std::str::from_utf8(&digits[..len]).ok()?.parse().ok()
        })
        .max()
}

struct Log {
    stream: u64,
    next: Option<u64>,
    gaps: usize,
    paints: u64,
    painted_at: Option<Instant>,
    paint_log: Vec<(Instant, u64)>,
    arrivals: Vec<(Instant, usize)>,
    tail: Vec<u8>,
    model: Option<Model>,
}

impl Log {
    fn scan(&mut self, bytes: &[u8]) {
        let mut buf = std::mem::take(&mut self.tail);
        let old = buf.len();
        buf.extend_from_slice(bytes);
        if let Some(paint) = painted_in(&buf, old).filter(|&paint| paint > self.paints) {
            self.paints = paint;
            self.painted_at = Some(Instant::now());
            self.paint_log.push((Instant::now(), paint));
        }
        self.tail = buf[buf.len().saturating_sub(16)..].to_vec();
    }
}

fn find<'a>(hay: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    hay.windows(needle.len())
        .enumerate()
        .filter(move |(_, window)| *window == needle)
        .map(|(at, _)| at)
}

/// A desktop from before frames, reading the raw bytes the program wrote; it answers nothing.
#[derive(Clone)]
struct RawDesktop {
    client: Arc<RpcClient>,
    terminal: String,
    log: Arc<Mutex<Log>>,
}

impl RawDesktop {
    fn new(client: Arc<RpcClient>, terminal: &str) -> Self {
        Self {
            client,
            terminal: terminal.to_string(),
            log: Arc::new(Mutex::new(Log {
                stream: 0,
                next: None,
                gaps: 0,
                paints: 0,
                painted_at: None,
                paint_log: Vec::new(),
                arrivals: Vec::new(),
                tail: Vec::new(),
                model: None,
            })),
        }
    }

    fn modelled(self, model: Model) -> Self {
        self.log.lock().unwrap().model = Some(model);
        self
    }

    fn watch(&self, mut stream: TerminalStream) {
        let me = self.clone();
        let generation = {
            let mut log = self.log.lock().unwrap();
            log.stream += 1;
            log.stream
        };
        tokio::spawn(async move {
            while let Some(event) = stream.recv().await {
                let mut log = me.log.lock().unwrap();
                if log.stream != generation {
                    return;
                }
                match event {
                    TerminalEvent::Resized { cols, rows, .. } => {
                        if let Some(model) = &log.model {
                            model.resize(cols, rows);
                        }
                    }
                    TerminalEvent::Clipboard { .. } => {}
                    TerminalEvent::Output { data, offset, .. } => {
                        let bytes = STANDARD.decode(data).expect("output is base64");
                        if matches!((offset, log.next), (Some(at), Some(next)) if at > next) {
                            log.gaps += 1;
                        }
                        log.next = offset.map(|at| at + bytes.len() as u64);
                        log.arrivals.push((Instant::now(), bytes.len()));
                        if let Some(model) = &log.model {
                            model.feed(&bytes);
                        }
                        log.scan(&bytes);
                    }
                    TerminalEvent::Exited { .. } => return,
                }
            }
        });
    }

    fn forget_stream(&self) {
        self.log.lock().unwrap().stream += 1;
    }

    async fn leave(&self, stream: u64) -> Option<u64> {
        let _ = self.client.cancel_stream(stream).await;
        self.forget_stream();
        self.log.lock().unwrap().next
    }

    async fn attach(&self, from: Option<u64>) -> (u64, bool) {
        let attached = self
            .client
            .terminal_attach_stream(self.terminal.clone(), from)
            .await
            .expect("terminal_attach")
            .expect("a live terminal");
        self.watch(attached.events);
        (attached.stream_id, attached.resumed)
    }

    async fn painted(&self, paint: u64) -> bool {
        eventually(WAIT, || self.paints() >= paint).await
    }

    fn gaps(&self) -> usize {
        self.log.lock().unwrap().gaps
    }

    fn paints(&self) -> u64 {
        self.log.lock().unwrap().paints
    }

    async fn repainted(&self, paints: u64) -> bool {
        self.painted(paints + 1).await && self.settled(Duration::from_millis(600)).await
    }

    async fn settled(&self, quiet: Duration) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut last = (self.paints(), Instant::now());
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let paints = self.paints();
            if paints != last.0 {
                last = (paints, Instant::now());
            } else if last.1.elapsed() >= quiet {
                return true;
            }
        }
        false
    }
}

/// The server's screen: the same libghostty, fed the same bytes at the same sizes as the tap is
/// sent, with the server's scrollback limit.
#[derive(Clone)]
struct Model(Arc<Mutex<Terminal>>);

#[derive(Clone, PartialEq)]
struct ModelCell {
    text: String,
    width: u8,
    fg: String,
    bg: String,
    flags: u64,
}

struct Snapshot {
    cols: u16,
    rows: u16,
    grid: Vec<Vec<ModelCell>>,
    continues: Vec<bool>,
    history: Vec<String>,
}

fn color(color: Option<CellColor>) -> String {
    match color {
        None => "d".into(),
        Some(CellColor::Palette(i)) => format!("p{i}"),
        Some(CellColor::Rgb(c)) => format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
    }
}

fn model_cell(text: String, wide: CellWide, style: &CellStyle, linked: bool) -> ModelCell {
    // xterm draws a linked cell underlined.
    let flags = [
        style.bold,
        style.faint,
        style.italic,
        style.underlined || linked,
        style.inverse,
        style.strikethrough,
        style.invisible,
        style.blink,
    ]
    .iter()
    .enumerate()
    .filter(|(_, on)| **on)
    .fold(0, |flags, (bit, _)| flags | 1 << bit);
    ModelCell {
        text,
        width: match wide {
            CellWide::SpacerTail => 0,
            CellWide::Wide => 2,
            _ => 1,
        },
        fg: color(style.fg_color),
        bg: color(style.bg_color),
        flags,
    }
}

impl Model {
    fn new(cols: u16, rows: u16) -> Self {
        let terminal = Terminal::new(cols, rows, 2 * 1024 * 1024).expect("a libghostty terminal");
        Self(Arc::new(Mutex::new(terminal)))
    }

    fn feed(&self, bytes: &[u8]) {
        let mut terminal = self.0.lock().unwrap();
        terminal.write(bytes);
        terminal.take_clipboard_writes();
    }

    fn resize(&self, cols: u16, rows: u16) {
        let _ = self.0.lock().unwrap().resize(cols, rows, 0, 0);
    }

    fn row(terminal: &Terminal, at: RowAt) -> (Vec<ModelCell>, bool) {
        let mut cells = Vec::new();
        let (_, continues) = terminal
            .for_each_row_cell(at, |cell| {
                let text = cell
                    .graphemes
                    .iter()
                    .filter_map(|&c| char::from_u32(c))
                    .collect();
                cells.push(model_cell(
                    text,
                    cell.wide,
                    &cell.style,
                    cell.hyperlink.is_some(),
                ));
            })
            .expect("a model row");
        (cells, continues)
    }

    fn snapshot(&self) -> Snapshot {
        let terminal = self.0.lock().unwrap();
        let (cols, rows) = (terminal.cols().unwrap(), terminal.rows().unwrap());
        let (grid, continues) = (0..rows)
            .map(|y| Self::row(&terminal, RowAt::Viewport(y.into())))
            .unzip();
        let held = terminal.scrollback_rows().unwrap() as u32;
        let history = (0..held)
            .map(|y| {
                Self::row(&terminal, RowAt::History(y))
                    .0
                    .iter()
                    .filter(|cell| cell.width > 0)
                    .map(|cell| cell.text.as_str())
                    .collect()
            })
            .collect();
        Snapshot {
            cols,
            rows,
            grid,
            continues,
            history,
        }
    }
}

/// The cells a frame client shows differently from the model. xterm 5.5 measures with Unicode 6,
/// so a cluster it spreads over exactly the model's two cells is not counted: the raw stream
/// draws it the same way.
fn mismatched(model: &Snapshot, view: &Value) -> usize {
    if view["cols"] != json!(model.cols) || view["rows"] != json!(model.rows) {
        return usize::from(model.cols) * usize::from(model.rows);
    }
    let blank = |s: &str| s.is_empty() || s == " ";
    let mut bad = 0;
    for (y, row) in model.grid.iter().enumerate() {
        let seen = &view["grid"][y];
        for (x, cell) in row.iter().enumerate() {
            if cell.width == 0 {
                continue;
            }
            let shown = &seen[x];
            let lead = shown[0].as_str().unwrap_or_default();
            let lead = if lead.is_empty() { " " } else { lead };
            let want = if cell.text.is_empty() {
                " "
            } else {
                cell.text.as_str()
            };
            if cell.width == 1 {
                bad += usize::from(lead != want);
            } else {
                let spacer = &seen[x + 1];
                let spacer_text = spacer[0].as_str().unwrap_or_default();
                if lead == want {
                    bad += usize::from(!blank(spacer_text) || spacer[3] != json!(cell.bg));
                } else if want.starts_with(lead) && spacer_text == &want[lead.len()..] {
                    continue;
                } else {
                    bad += 1;
                }
            }
            bad += usize::from(
                shown[2] != json!(cell.fg)
                    || shown[3] != json!(cell.bg)
                    || shown[4] != json!(cell.flags),
            );
        }
    }
    bad
}

/// Whether the client's history ends as the model's does, line for line. Spaces are dropped
/// from both, so an emoji xterm draws narrow beside a blank cell reads the same.
fn history_matches(model: &Snapshot, view: &Value) -> bool {
    let squash = |line: &str| line.split_whitespace().collect::<String>();
    let client: Vec<String> = view["history"]
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .map(|l| squash(l.as_str().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let model: Vec<String> = model.history.iter().map(|l| squash(l)).collect();
    let n = client.len().min(model.len());
    client[client.len() - n..] == model[model.len() - n..]
}

fn wraps_match(model: &Snapshot, view: &Value) -> bool {
    let wrapped = view["wrapped"].as_array().cloned().unwrap_or_default();
    let screen = &wrapped[wrapped.len().saturating_sub(model.continues.len())..];
    screen
        .iter()
        .map(|w| w.as_bool())
        .eq(model.continues.iter().map(|c| Some(*c)))
}

/// How many cells and wrap flags `screen` holds differently from the model, whether its history
/// ends as the model's does, and the first difference. Exact, but a blank cell is a space or
/// empty alike: frames paint the spaces the program wrote.
fn native_mismatched(model: &Snapshot, screen: &FrameScreen) -> (usize, bool, String) {
    let all = usize::from(model.cols) * usize::from(model.rows);
    let Ok(rows) = screen.rows() else {
        return (all, false, "unreadable".into());
    };
    if screen.size().ok() != Some((model.cols, model.rows)) || rows.len() != model.grid.len() {
        return (all, false, format!("size {:?}", screen.size().ok()));
    }
    let (mut bad, mut first) = (0, String::new());
    for (y, ((cells, continues), (want, want_continues))) in rows
        .iter()
        .zip(model.grid.iter().zip(&model.continues))
        .enumerate()
    {
        if continues != want_continues {
            bad += 1;
            first = format!("row {y} wrap {continues} want {want_continues}");
        }
        if cells.len() != want.len() {
            bad += want.len();
            continue;
        }
        for (x, (cell, want)) in cells.iter().zip(want).enumerate() {
            let (mut got, mut want_cell) = (mapped(cell), want.clone());
            for c in [&mut got, &mut want_cell] {
                if c.text.is_empty() {
                    c.text = " ".into();
                }
            }
            if got != want_cell {
                bad += 1;
                first = format!("cell {x},{y}: {:?} want {:?}", got.text, want_cell.text);
            }
        }
    }
    // The text alone, tails aligned: a reflow moves where rows break, and blank rows come and go.
    let squash = |lines: &[String]| -> Vec<char> {
        lines
            .iter()
            .flat_map(|l| l.chars())
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    let (history, model_history) = (
        squash(&screen.history().unwrap_or_default()),
        squash(&model.history),
    );
    let n = history.len().min(model_history.len());
    let (held, kept) = (
        &history[history.len() - n..],
        &model_history[model_history.len() - n..],
    );
    let ends_alike = held == kept;
    if !ends_alike {
        let at = held.iter().zip(kept).position(|(g, w)| g != w).unwrap_or(0);
        let (lo, hi) = (at.saturating_sub(20), (at + 40).min(n));
        first = format!(
            "history char {at}/{n}: client {:?} model {:?}",
            held[lo..hi].iter().collect::<String>(),
            kept[lo..hi].iter().collect::<String>()
        );
    }
    (
        bad,
        ends_alike && (n > 0 || model_history.is_empty()),
        first,
    )
}

fn mapped(cell: &ScreenCell) -> ModelCell {
    model_cell(cell.text.clone(), cell.wide, &cell.style, cell.linked)
}

/// This thread's CPU time, for work that doesn't await.
fn thread_cpu() -> Duration {
    let mut at = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid out pointer.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut at) };
    Duration::new(at.tv_sec as u64, at.tv_nsec as u32)
}

#[derive(Clone)]
struct Xterm(
    mpsc::UnboundedSender<(String, Option<oneshot::Sender<Value>>)>,
    Option<u32>,
);

impl Xterm {
    fn spawn() -> Self {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/xterm/render.mjs");
        let mut child = tokio::process::Command::new("node")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("`node` runs xterm/render.mjs (and `pnpm install` gives it @xterm/headless)");
        let pid = child.id();
        let mut stdin = child.stdin.take().expect("node's stdin");
        let mut lines = BufReader::new(child.stdout.take().expect("node's stdout")).lines();
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, Option<oneshot::Sender<Value>>)>();
        tokio::spawn(async move {
            let _child = child;
            while let Some((command, reply)) = rx.recv().await {
                if stdin
                    .write_all(format!("{command}\n").as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                if let Some(reply) = reply {
                    let line = lines.next_line().await.ok().flatten().unwrap_or_default();
                    let _ = reply.send(serde_json::from_str(&line).unwrap_or(Value::Null));
                }
            }
        });
        Self(tx, pid)
    }

    /// The node process's CPU time so far, read with `ps`.
    fn cpu(&self) -> Duration {
        let ps = std::process::Command::new("ps")
            .args([
                "-o",
                "cputime=",
                "-p",
                &self.1.unwrap_or_default().to_string(),
            ])
            .output()
            .ok();
        // `[[dd-]hh:]mm:ss[.cc]`.
        let text = ps.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let secs = text
            .unwrap_or_default()
            .split(':')
            .try_fold(0.0, |acc, part| {
                part.rsplit('-')
                    .next()
                    .and_then(|p| p.parse::<f64>().ok())
                    .map(|n| acc * 60.0 + n)
            });
        Duration::from_secs_f64(secs.unwrap_or(0.0))
    }

    fn send(&self, command: Value) {
        let _ = self.0.send((command.to_string(), None));
    }

    async fn dump(&self) -> Value {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send((json!({ "dump": true }).to_string(), Some(tx)));
        rx.await.unwrap_or(Value::Null)
    }
}

#[derive(Default)]
struct FrameLog {
    stream: u64,
    history: Option<u64>,
    raw: usize,
    copies: Vec<String>,
    frames: usize,
    bytes: usize,
    paints: u64,
    paint_log: Vec<(Instant, u64)>,
    first_frame: Option<(Instant, usize)>,
    last_frame: Option<Instant>,
}

/// A desktop as the app is one: it writes each frame into xterm 5.5 and acks it once handed over.
#[derive(Clone)]
struct FrameDesktop {
    client: Arc<RpcClient>,
    terminal: String,
    xterm: Xterm,
    screen: Arc<Mutex<FrameScreen>>,
    /// Time spent writing frames into `screen`.
    native_cpu: Arc<Mutex<Duration>>,
    /// Set once this client is narrower than the history rows it was sent: it then lacks their text past its width.
    narrowed: Arc<AtomicBool>,
    log: Arc<Mutex<FrameLog>>,
}

impl FrameDesktop {
    fn new(client: Arc<RpcClient>, terminal: &str) -> Self {
        Self {
            client,
            terminal: terminal.to_string(),
            xterm: Xterm::spawn(),
            screen: Arc::new(Mutex::new(
                FrameScreen::new(80, 24).expect("a frame screen"),
            )),
            native_cpu: Arc::default(),
            narrowed: Arc::default(),
            log: Arc::default(),
        }
    }

    async fn attach(&self, from: Option<u64>) -> (u64, bool) {
        let attached = self
            .client
            .terminal_attach_stream(self.terminal.clone(), from)
            .await
            .expect("terminal_attach")
            .expect("a live terminal");
        self.watch(attached.stream_id, attached.events);
        (attached.stream_id, attached.resumed)
    }

    fn watch(&self, stream_id: u64, mut stream: TerminalStream) {
        let generation = {
            let mut log = self.log.lock().unwrap();
            log.stream += 1;
            log.stream
        };
        let me = self.clone();
        tokio::spawn(async move {
            while let Some(event) = stream.recv().await {
                let mut log = me.log.lock().unwrap();
                if log.stream != generation {
                    return;
                }
                match event {
                    TerminalEvent::Resized { cols, rows, .. } => {
                        me.xterm.send(json!({ "resize": [cols, rows] }));
                        let _ = me.screen.lock().unwrap().resize(cols, rows);
                    }
                    TerminalEvent::Output {
                        data,
                        offset,
                        frame: Some(seq),
                    } => {
                        let bytes = STANDARD.decode(&data).unwrap_or_default();
                        log.frames += 1;
                        log.bytes += bytes.len();
                        log.first_frame.get_or_insert((Instant::now(), bytes.len()));
                        log.last_frame = Some(Instant::now());
                        if let Some(paint) = painted_in(&bytes, 0).filter(|&p| p > log.paints) {
                            log.paints = paint;
                            log.paint_log.push((Instant::now(), paint));
                        }
                        me.xterm.send(json!({ "write": data }));
                        let started = thread_cpu();
                        me.screen.lock().unwrap().write(&bytes);
                        *me.native_cpu.lock().unwrap() += thread_cpu() - started;
                        log.history = offset;
                        let client = me.client.clone();
                        tokio::spawn(async move {
                            let _ = client.terminal_frame_ack(stream_id, seq).await;
                        });
                    }
                    TerminalEvent::Output { .. } => log.raw += 1,
                    TerminalEvent::Clipboard { data } => {
                        let text = STANDARD.decode(data).unwrap_or_default();
                        log.copies.push(String::from_utf8_lossy(&text).into_owned());
                    }
                    TerminalEvent::Exited { .. } => return,
                }
            }
        });
    }

    async fn leave(&self, stream: u64) -> Option<u64> {
        let _ = self.client.cancel_stream(stream).await;
        let mut log = self.log.lock().unwrap();
        log.stream += 1;
        log.history
    }

    async fn send(&self, text: &str) {
        self.client
            .terminal_write(self.terminal.clone(), keystrokes(text))
            .await
            .expect("terminal_write");
    }

    async fn fit(&self, cols: u16, rows: u16) {
        self.client
            .terminal_resize(self.terminal.clone(), cols, rows)
            .await
            .expect("terminal_resize");
    }

    async fn theme(&self, theme: TerminalTheme) {
        self.client
            .terminal_theme(theme)
            .await
            .expect("terminal_theme");
    }

    async fn focus(&self) {
        self.client
            .terminal_focus(self.terminal.clone())
            .await
            .expect("terminal_focus");
    }

    async fn take(&self) {
        self.client
            .terminal_take_size(self.terminal.clone())
            .await
            .expect("terminal_take_size");
    }

    fn copies(&self) -> Vec<String> {
        self.log.lock().unwrap().copies.clone()
    }

    fn paints(&self) -> u64 {
        self.log.lock().unwrap().paints
    }

    async fn painted(&self, paint: u64) -> bool {
        eventually(WAIT, || self.paints() >= paint).await
    }

    async fn shows(&self, model: &Model) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (snapshot, view) = (model.snapshot(), self.xterm.dump().await);
            let bad = mismatched(&snapshot, &view);
            let history = history_matches(&snapshot, &view);
            let (native_bad, native_history, native_first) =
                native_mismatched(&snapshot, &self.screen.lock().unwrap());
            let lossy = self.narrowed.load(Relaxed);
            let done = bad == 0 && history && native_bad == 0 && (native_history || lossy);
            if done || Instant::now() >= deadline {
                let raw = self.log.lock().unwrap().raw;
                if !done {
                    eprintln!("frame screen differs: {native_first}");
                }
                let evidence = json!({
                    "grids_match": bad == 0,
                    "mismatched_cells": bad,
                    "history_matches": history,
                    "frame_screen_grids_match": native_bad == 0,
                    "frame_screen_mismatched_cells": native_bad,
                    "frame_screen_history_matches": native_history,
                    "raw_output_events": raw,
                });
                return if lossy {
                    with(
                        evidence,
                        json!({ "frame_screen_history_known_lossy": true }),
                    )
                } else {
                    evidence
                };
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

const SHOWS: &str = "the frame client shows the server's screen, cells and history";

fn frames_ok(evidence: &Value) -> bool {
    evidence["grids_match"] == json!(true)
        && evidence["history_matches"] == json!(true)
        && evidence["frame_screen_grids_match"] == json!(true)
        && (evidence["frame_screen_history_matches"] == json!(true)
            || evidence["frame_screen_history_known_lossy"] == json!(true))
        && evidence["raw_output_events"] == json!(0)
}

async fn compare(a: &FrameDesktop, b: &FrameDesktop, answers_seen: u64) -> Value {
    let (a, b) = (a.xterm.dump().await, b.xterm.dump().await);
    let size = |key: &str| a[key].as_u64().max(b[key].as_u64()).unwrap_or(0) as usize;
    let mismatched = (0..size("rows"))
        .flat_map(|y| (0..size("cols")).map(move |x| (y, x)))
        .filter(|&(y, x)| a["grid"][y][x] != b["grid"][y][x])
        .count();
    json!({
        "grids_match": mismatched == 0 && a["cols"] == b["cols"] && a["rows"] == b["rows"],
        "mismatched_cells": mismatched,
        "answers_seen": answers_seen,
    })
}

fn lines(view: &Value) -> Vec<String> {
    let row = |cells: &Value| -> String {
        let text: String = cells
            .as_array()
            .into_iter()
            .flatten()
            .filter(|cell| cell[1] != json!(0))
            .map(|cell| match cell[0].as_str() {
                Some("") | None => " ",
                Some(text) => text,
            })
            .collect();
        text.trim_end().to_string()
    };
    view["grid"]
        .as_array()
        .into_iter()
        .flatten()
        .map(row)
        .collect()
}

async fn reported(desktop: &FrameDesktop, label: &str) -> Value {
    let lines = lines(&desktop.xterm.dump().await);
    let line = lines
        .iter()
        .find_map(|line| line.find(label).map(|at| &line[at..]))
        .unwrap_or_default();
    let counts: serde_json::Map<String, Value> = line
        .split_whitespace()
        .filter_map(|pair| pair.split_once('='))
        .filter_map(|(key, n)| Some((key.to_string(), json!(n.parse::<u64>().ok()?))))
        .collect();
    Value::Object(counts)
}

async fn show(j: &mut Journey, case: &str, a: &FrameDesktop, b: &FrameDesktop) {
    let (a, b) = (lines(&a.xterm.dump().await), lines(&b.xterm.dump().await));
    j.note(&format!("screens {case}"), json!({ "a": a, "b": b }));
}

fn with(mut evidence: Value, extra: Value) -> Value {
    if let (Some(evidence), Value::Object(extra)) = (evidence.as_object_mut(), extra) {
        evidence.extend(extra);
    }
    evidence
}

const DREW: &str = "the tap saw every byte and both desktops drew the repaint";

async fn frames_shown(j: &mut Journey, c: &FrameDesktop, model: &Model) {
    let evidence = c.shows(model).await;
    j.guard(SHOWS, frames_ok(&evidence), evidence);
}

/// The client's history still starts with `kept`, line breaks aside: a reattach didn't start it over.
fn history_kept(kept: &Value, now: &Value) -> bool {
    let text = |lines: &Value| -> String {
        lines
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .flat_map(str::split_whitespace)
            .collect()
    };
    text(now).starts_with(&text(kept))
}

fn has_line(view: &Value, line: &str) -> bool {
    let want: String = line.split_whitespace().collect();
    view["history"].as_array().into_iter().flatten().any(|l| {
        l.as_str()
            .is_some_and(|l| l.split_whitespace().collect::<String>() == want)
    })
}

async fn drawn(tap: &RawDesktop, a: &FrameDesktop, b: &FrameDesktop, paint: u64) -> bool {
    tap.painted(paint).await && a.painted(paint).await && b.painted(paint).await && tap.gaps() == 0
}

fn stub_command(sandbox: &Sandbox) -> String {
    let stub = install(sandbox, "screen-tui", SCREEN_TUI);
    format!(
        "'{}' '{FIXTURE}' '{}'",
        stub.display(),
        winches_log(sandbox).display()
    )
}

fn winches_log(sandbox: &Sandbox) -> PathBuf {
    sandbox.root.join("winches.log")
}

/// Each size the program was resized to, as `rows cols`.
fn winches(sandbox: &Sandbox) -> Vec<String> {
    std::fs::read_to_string(winches_log(sandbox))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_terminal_screen() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("shared_terminal_screen", &sandbox);
    let server = Server::start(&sandbox).await;
    let (b_client, b_link) = far_phone(server.socket(), true, Duration::ZERO, false).await;

    j.step("A opens the program at 120x40 and B attaches at 80x24");
    // As the app does at start: a pty takes its spawner's theme.
    server
        .client()
        .terminal_theme(A_THEME)
        .await
        .expect("A reports its theme");
    let (terminal, mut a_stream, opened) = server
        .client()
        .spawn_shell_terminal_stream(stub_command(&sandbox), 120, 40)
        .await
        .expect("spawn the stub");
    let a = FrameDesktop::new(server.client(), &terminal);
    a.watch(a_stream, opened);
    a.fit(120, 40).await;
    let model = Model::new(120, 40);
    let tap = RawDesktop::new(server.connect_announcing(RAW_DESKTOP).await, &terminal)
        .modelled(model.clone());
    let (mut tap_stream, _) = tap.attach(None).await;
    let opened = tap.painted(1).await;
    let b = FrameDesktop::new(b_client, &terminal);
    b.theme(LIGHT_THEME).await;
    let (mut b_stream, _) = b.attach(None).await;
    b.fit(80, 24).await;
    let (c_client, c_link) = far_phone_announcing(
        server.socket(),
        true,
        Duration::ZERO,
        false,
        Some(Features::ALL),
    )
    .await;
    let c = FrameDesktop::new(c_client, &terminal);
    c.theme(A_THEME).await;
    let (mut c_stream, _) = c.attach(None).await;
    let ok = opened && drawn(&tap, &a, &b, 1).await;
    j.guard(DREW, ok, compare(&a, &b, 0).await);
    show(&mut j, "attach", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("(a) A resizes, and B's typing and focus leave the size with A");
    a.fit(100, 30).await;
    let ok = drawn(&tap, &a, &b, 2).await;
    b.focus().await;
    b.send("j").await;
    let kept = drawn(&tap, &a, &b, 3).await && tap.settled(Duration::from_millis(1500)).await;
    let (a_view, b_view) = (a.xterm.dump().await, b.xterm.dump().await);
    let size = |view: &Value| json!([view["cols"], view["rows"]]);
    j.guard(
        DREW,
        ok && kept && size(&a_view) == json!([100, 30]) && size(&b_view) == json!([100, 30]),
        with(
            compare(&a, &b, 0).await,
            json!({ "size": size(&b_view), "paints": tap.paints() }),
        ),
    );
    show(&mut j, "(a)", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("(b) B takes the size, then resizes");
    b.take().await;
    let typed = drawn(&tap, &a, &b, 4).await;
    b.fit(90, 28).await;
    let ok = typed && drawn(&tap, &a, &b, 5).await;
    j.guard(DREW, ok, compare(&a, &b, 0).await);
    show(&mut j, "(b)", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("(c) B answers the picker, which erases and redraws it");
    b.send("\r").await;
    let ok = drawn(&tap, &a, &b, 6).await;
    j.guard(DREW, ok, compare(&a, &b, 0).await);
    show(&mut j, "(c)", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("(d) the program asks for the theme while B, the light desktop, is active");
    // A raw viewer leaves the program's queries to the raw desktops, so the tap steps out first.
    let from = tap.leave(tap_stream).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    b.send("t").await;
    let drew = a.painted(7).await && b.painted(7).await;
    let (stream, tap_resumed) = tap.attach(from).await;
    tap_stream = stream;
    let ok = drew && tap_resumed && drawn(&tap, &a, &b, 7).await;
    let answers = reported(&a, "ANSWERS").await;
    let osc11 = answers["osc11"].as_u64().unwrap_or_default();
    j.guard(
        DREW,
        ok,
        with(
            compare(&a, &b, osc11).await,
            json!({
                "scheme_answers": answers["scheme"],
                "themes_heard": { "dark": answers["dark"], "light": answers["light"] },
                "active": "light",
            }),
        ),
    );
    show(&mut j, "(d)", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("Ctrl+F3 and a key batched with a reply reach the program");
    b.send("h").await;
    let listening = drawn(&tap, &a, &b, 8).await;
    b.send("\x1b[1;5R").await;
    b.send(&format!("x{LIGHT_REPLY}")).await;
    let ok = listening && drawn(&tap, &a, &b, 9).await;
    let heard = reported(&a, "HEARD").await;
    j.guard(
        DREW,
        ok,
        with(
            compare(&a, &b, 0).await,
            json!({ "ctrl_f3_heard": heard["ctrl_f3"], "batched_key_heard": heard["batched_key"] }),
        ),
    );
    show(&mut j, "ctrl+f3", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("(e) B's link stalls for 5 s while the program writes, then B reads");
    b_link.down_stalled.store(true, Relaxed);
    let stalled_at = Instant::now();
    let typing = {
        let b = b.clone();
        tokio::spawn(async move { b.send("F").await })
    };
    let a_drew = tap.painted(10).await && a.painted(10).await;
    tokio::time::sleep_until((stalled_at + STALL).into()).await;
    b_link.down_stalled.store(false, Relaxed);
    let _ = typing.await;
    let ok = a_drew && drawn(&tap, &a, &b, 10).await;
    j.guard(DREW, ok, compare(&a, &b, 0).await);
    show(&mut j, "(e)", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("(f) B reconnects after missing more output than the terminal keeps");
    let from = b.leave(b_stream).await;
    let size_back = tap.painted(11).await;
    a.send("F").await;
    let flooded = tap.painted(12).await;
    let (stream, resumed) = b.attach(from).await;
    b_stream = stream;
    b.fit(80, 24).await;
    let ok = size_back && flooded && drawn(&tap, &a, &b, 12).await;
    j.guard(
        DREW,
        ok,
        with(compare(&a, &b, 0).await, json!({ "resumed": resumed })),
    );
    show(&mut j, "(f)", &a, &b).await;
    frames_shown(&mut j, &c, &model).await;

    j.step("frames: C draws a row of emoji, CJK and a flag on a coloured picker row");
    let paints = tap.paints();
    c.fit(110, 32).await;
    c.take().await;
    c.send("E").await;
    let settled = tap.repainted(paints).await;
    let evidence = c.shows(&model).await;
    j.guard(SHOWS, settled && frames_ok(&evidence), evidence);

    j.step("frames: a URL the program wrote past the right margin stays one wrapped line");
    let paints = tap.paints();
    c.send("U").await;
    let settled = tap.repainted(paints).await;
    let evidence = c.shows(&model).await;
    let (snapshot, view) = (model.snapshot(), c.xterm.dump().await);
    let wrapped = snapshot.continues.iter().any(|c| *c) && wraps_match(&snapshot, &view);
    j.guard(
        "the frame client shows the screen, and xterm joins the URL's rows as the model does",
        settled && frames_ok(&evidence) && wrapped,
        with(evidence, json!({ "url_rows_wrapped": wrapped })),
    );

    j.step(
        "frames: C's link stalls while 300 lines scroll past, a progress line finished among them",
    );
    let paints = tap.paints();
    c_link.down_stalled.store(true, Relaxed);
    // Its answer can't come back until the link does.
    let typing = {
        let c = c.clone();
        tokio::spawn(async move { c.send("L").await })
    };
    let settled = tap.repainted(paints).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    c_link.down_stalled.store(false, Relaxed);
    let _ = typing.await;
    let evidence = c.shows(&model).await;
    let view = c.xterm.dump().await;
    let finished = has_line(&view, "progress 100% done") && !has_line(&view, "progress 10%");
    j.guard(
        "after the stall the client's history holds every line as the model has it, the finished progress line included",
        settled && frames_ok(&evidence) && finished,
        with(evidence, json!({ "progress_line_finished": finished })),
    );

    j.step("frames: C narrows to 60 columns and back, so both sides reflow that history");
    let mut reflowed = Vec::new();
    for (cols, rows) in [(60, 30), (110, 32)] {
        c.narrowed.store(true, Relaxed);
        let paints = tap.paints();
        c.fit(cols, rows).await;
        let settled = tap.repainted(paints).await;
        let evidence = c.shows(&model).await;
        reflowed.push(json!({ "settled": settled, "cols": cols, "shows": evidence }));
    }
    let ok = reflowed
        .iter()
        .all(|r| r["settled"] == json!(true) && frames_ok(&r["shows"]));
    j.guard(
        "after each resize the client's history text is the model's",
        ok,
        json!({ "resizes": reflowed }),
    );

    j.step(
        "frames: C drops its stream, the program writes, and C reattaches from its history count",
    );
    let kept = c.xterm.dump().await["history"].clone();
    let cursor = c.leave(c_stream).await;
    let paints = tap.paints();
    c.send("j").await;
    let drew = tap.repainted(paints).await;
    let (stream, resumed) = c.attach(cursor).await;
    c_stream = stream;
    let evidence = c.shows(&model).await;
    let kept = history_kept(&kept, &c.xterm.dump().await["history"]);
    j.guard(
        "the reattach resumes: the client keeps its history and shows the screen",
        drew && resumed && kept && frames_ok(&evidence),
        with(
            evidence,
            json!({ "resumed": resumed, "history_kept": kept }),
        ),
    );

    j.step("frames: the program copies text while C owns the size and D watches");
    let d = FrameDesktop::new(server.connect_announcing(Features::ALL).await, &terminal);
    d.theme(LIGHT_THEME).await;
    let (d_stream, _) = d.attach(None).await;
    d.fit(72, 40).await;
    d.narrowed.store(true, Relaxed);
    c.fit(124, 40).await;
    c.take().await;
    let paints = tap.paints();
    c.send("C").await;
    let settled = tap.repainted(paints).await;
    let owner = c
        .copies()
        .iter()
        .any(|copy| copy == "copied by the program");
    let others = [&a, &b, &d]
        .iter()
        .any(|desktop| !desktop.copies().is_empty());
    let (c_shows, d_shows) = (c.shows(&model).await, d.shows(&model).await);
    j.guard(
        "only the size owner's frame stream carries the copy",
        settled && owner && !others && frames_ok(&c_shows) && frames_ok(&d_shows),
        json!({ "owner_copied": owner, "other_copied": others, "c": c_shows, "d": d_shows }),
    );

    j.step("frames: the size flips between C at 124 and D at 72 columns, the program redrawing everything each time");
    let paints = tap.paints();
    c.send("W").await;
    let mut redrawn = tap.repainted(paints).await;
    let flips = 20;
    for flip in 0..flips {
        if flip % 2 == 0 {
            d.take().await
        } else {
            c.take().await
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    redrawn &= tap.settled(Duration::from_millis(1500)).await;
    let (c_shows, d_shows) = (c.shows(&model).await, d.shows(&model).await);
    let paints = tap.paints();
    c.send("w").await;
    redrawn &= tap.repainted(paints).await;
    j.guard(
        "both frame desktops show the server's screen and history after the flips",
        redrawn && frames_ok(&c_shows) && frames_ok(&d_shows),
        json!({ "flips": flips, "c": c_shows, "d": d_shows }),
    );

    j.step("(g) with nobody watching, the program asks for the theme");
    let paints = tap.paints();
    tap.leave(tap_stream).await;
    for (desktop, stream) in [(&a, a_stream), (&b, b_stream), (&d, d_stream)] {
        desktop.leave(stream).await;
    }
    // The size passes to the next viewer as each leaves, so C, which owns it, leaves last.
    tokio::time::sleep(Duration::from_secs(1)).await;
    c.leave(c_stream).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    a.send("t").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    (a_stream, _) = a.attach(None).await;
    let painted = a.painted(paints + 1).await;
    let answers = reported(&a, "ANSWERS").await;
    j.guard(
        "the server answers each query once, in the theme of the desktop that last owned the size",
        painted
            && answers["osc11"] == json!(1)
            && answers["scheme"] == json!(1)
            && answers["dark"] == json!(1)
            && answers["light"] == json!(0),
        json!({ "answers": answers }),
    );

    j.step("A reattaches and asks for an agent fit at the size the terminal already has");
    let paints = a.paints();
    a.fit(96, 30).await;
    let fitted = a.painted(paints + 1).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let paints = a.paints();
    a.client
        .agent_resize(terminal.clone(), 96, 30)
        .await
        .expect("agent_resize");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    j.guard(
        "the program is not resized, so it does not repaint",
        fitted && a.paints() == paints,
        json!({ "repaints": a.paints() - paints }),
    );

    j.step("D opens the agent while A owns the size, then A's agent tab changes size");
    let (d_stream, _) = d.attach(None).await;
    let before = winches(&sandbox).len();
    d.client
        .agent_resize(terminal.clone(), 70, 20)
        .await
        .expect("agent_resize");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let opened_elsewhere = winches(&sandbox)[before..].to_vec();
    let before = winches(&sandbox).len();
    a.client
        .agent_resize(terminal.clone(), 100, 32)
        .await
        .expect("agent_resize");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let resized = winches(&sandbox)[before..].to_vec();
    j.guard(
        "the program is not resized for D, and A's new size reaches it in one resize, with no nudge first",
        opened_elsewhere.is_empty() && resized == ["32 100"],
        json!({ "for_d": opened_elsewhere, "for_a": resized }),
    );

    d.leave(d_stream).await;
    a.leave(a_stream).await;
    let _ = server.client().terminal_close(terminal).await;
    let cpu = frame_cpu(&[&a, &b, &c, &d]).await;
    j.note("frame_cpu", cpu.clone());
    j.finish();
    save_frame_cpu(&cpu);
}

/// Report only: the CPU `FrameScreen` and xterm-headless spent on the same frames.
async fn frame_cpu(desktops: &[&FrameDesktop]) -> Value {
    let idle = Xterm::spawn();
    idle.dump().await;
    let idle = idle.cpu();
    let native: Duration = desktops.iter().map(|d| *d.native_cpu.lock().unwrap()).sum();
    // Includes the dumps the comparisons asked for, less node's start-up.
    let xterm: Duration = desktops
        .iter()
        .map(|d| d.xterm.cpu().saturating_sub(idle))
        .sum();
    let frames: usize = desktops.iter().map(|d| d.log.lock().unwrap().frames).sum();
    let bytes: usize = desktops.iter().map(|d| d.log.lock().unwrap().bytes).sum();
    json!({
        "desktops": desktops.len(),
        "frames": frames,
        "frame_bytes": bytes,
        "frame_screen_cpu_ms": native.as_secs_f64() * 1000.0,
        "xterm_headless_cpu_ms": xterm.as_secs_f64() * 1000.0,
    })
}

/// The report scrubs times, so the figures go beside it.
fn save_frame_cpu(cpu: &Value) {
    let exe = std::env::current_exe().expect("the test executable");
    let dir = exe.ancestors().nth(3).expect("the target dir");
    let dir = dir.join("e2e/shared_terminal_screen");
    let _ = std::fs::write(dir.join("frame_cpu.json"), format!("{cpu:#}\n"));
    eprintln!("frame cpu: {cpu}");
}

#[cfg(target_os = "linux")]
mod baseline {
    use super::*;
    use hyprlayer_e2e::link::Link;

    struct Shape {
        name: &'static str,
        one_way: Duration,
        bps: u64,
    }

    const SHAPES: [Shape; 2] = [
        Shape {
            name: "11 ms RTT, 30 MB/s",
            one_way: Duration::from_micros(5500),
            bps: 30_000_000,
        },
        Shape {
            name: "60 ms RTT, 1 Mbit/s",
            one_way: Duration::from_millis(30),
            bps: 125_000,
        },
    ];
    const IDLE_PTYS: usize = 20;
    const REPAINTS: u64 = 580;
    // USER_HZ, the unit of /proc/<pid>/stat's times, is 100 on every Linux build.
    const USER_HZ: u64 = 100;

    async fn linked(
        server: &Server,
        shape: &Shape,
        features: Features,
    ) -> (Arc<RpcClient>, Arc<Link>) {
        let (client, link) =
            far_phone_announcing(server.socket(), true, shape.one_way, false, Some(features)).await;
        link.down_bps.store(shape.bps, Relaxed);
        link.up_bps.store(shape.bps, Relaxed);
        (client, link)
    }

    fn proc_file(pid: u32, file: &str) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/{file}")).unwrap_or_default()
    }

    fn rss_kib(pid: u32) -> u64 {
        proc_file(pid, "status")
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
            .unwrap_or_default()
    }

    fn cpu_ticks(pid: u32) -> u64 {
        let stat = proc_file(pid, "stat");
        let after_name = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
        // utime and stime are fields 14 and 15; this slice starts at field 3.
        after_name
            .split_whitespace()
            .skip(11)
            .take(2)
            .filter_map(|field| field.parse::<u64>().ok())
            .sum()
    }

    fn ms(d: Duration) -> f64 {
        (d.as_secs_f64() * 10_000.0).round() / 10.0
    }

    const ECHOES: u64 = 5;

    // Each key repaints the stub's picker once while its load runs.
    async fn type_during_load(client: &Arc<RpcClient>, terminal: &str) -> Vec<Instant> {
        let mut typed = Vec::new();
        for _ in 0..ECHOES {
            let (client, terminal) = (client.clone(), terminal.to_string());
            typed.push(Instant::now());
            tokio::spawn(async move { client.terminal_write(terminal, keystrokes("k")).await });
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        typed
    }

    fn reached(paint_log: &[(Instant, u64)], paint: u64) -> Option<Instant> {
        paint_log
            .iter()
            .find(|(_, p)| *p >= paint)
            .map(|(at, _)| *at)
    }

    fn echo_ms(paint_log: &[(Instant, u64)], before: u64, typed: &[Instant]) -> Vec<Option<f64>> {
        (1..)
            .zip(typed)
            .map(|(k, at)| reached(paint_log, before + k).map(|seen| ms(seen - *at)))
            .collect()
    }

    const HISTORY_SHELL: &str = "seq -f 'history line %04g the quick brown fox jumps over the lazy dog 0123456789' 3000; printf '(paint 1)'; exec sleep 600";

    async fn shell_history(server: &Server, shape: &Shape) -> Value {
        let raw = server.connect_announcing(RAW_DESKTOP).await;
        let (terminal, stream_id, opened) = raw
            .spawn_shell_terminal_stream(HISTORY_SHELL.into(), 120, 40)
            .await
            .expect("spawn the shell");
        let observer = RawDesktop::new(raw.clone(), &terminal);
        observer.watch(opened);
        let printed = observer.painted(1).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (client, _link) = linked(server, shape, Features::ALL).await;
        let c = FrameDesktop::new(client, &terminal);
        let asked = Instant::now();
        let (c_stream, _) = c.attach(None).await;
        let shown = c.painted(1).await;
        let screen = c
            .log
            .lock()
            .unwrap()
            .paint_log
            .first()
            .map(|(at, _)| ms(*at - asked));
        let mut last = (c.received(), Instant::now());
        while last.1.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if c.received() != last.0 {
                last = (c.received(), Instant::now());
            }
        }
        let (first_frame, history_done) = {
            let log = c.log.lock().unwrap();
            (log.first_frame, log.last_frame.map(|at| ms(at - asked)))
        };
        let history_rows = c.xterm.dump().await["history"]
            .as_array()
            .map_or(0, Vec::len);
        c.leave(c_stream).await;
        let _ = raw.cancel_stream(stream_id).await;
        let _ = raw.terminal_close(terminal).await;
        json!({
            "printed": printed && shown,
            "first_frame_ms": first_frame.map(|(at, _)| ms(at - asked)),
            "first_frame_bytes": first_frame.map(|(_, bytes)| bytes),
            "screen_ms": screen,
            "history_done_ms": history_done,
            "frames": last.0 .0,
            "bytes": last.0 .1,
            "history_rows": history_rows,
        })
    }

    impl RawDesktop {
        fn received(&self) -> (usize, usize) {
            let log = self.log.lock().unwrap();
            (
                log.arrivals.len(),
                log.arrivals.iter().map(|(_, n)| n).sum(),
            )
        }

        async fn send(&self, text: &str) {
            self.client
                .terminal_write(self.terminal.clone(), keystrokes(text))
                .await
                .expect("terminal_write");
        }

        async fn fit(&self, cols: u16, rows: u16) {
            self.client
                .terminal_resize(self.terminal.clone(), cols, rows)
                .await
                .expect("terminal_resize");
        }
    }

    async fn repaint_bytes(
        desktop: &RawDesktop,
        link: &Link,
        key: &str,
        paint: u64,
    ) -> Option<Value> {
        let (_, payload) = desktop.received();
        let wire = link.delivered.load(Relaxed);
        desktop.send(key).await;
        desktop.painted(paint).await.then(|| {
            json!({
                "payload_bytes": desktop.received().1 - payload,
                "wire_bytes": link.delivered.load(Relaxed) - wire,
            })
        })
    }

    async fn measure(j: &mut Journey, server: &Server, command: &str, shape: &Shape) -> Value {
        let pid = server.pid();
        let (client, link) = linked(server, shape, RAW_DESKTOP).await;
        let (terminal, stream_id, opened) = client
            .spawn_shell_terminal_stream(command.into(), 120, 40)
            .await
            .expect("spawn the stub");
        let desktop = RawDesktop::new(client.clone(), &terminal);
        desktop.watch(opened);
        desktop.fit(120, 40).await;
        let opened = desktop.painted(1).await;

        let picker = repaint_bytes(&desktop, &link, "j", 2).await;
        let scroll = repaint_bytes(&desktop, &link, "S", 3).await;
        j.guard(
            "a picker redraw and a 60-line scroll arrive",
            opened && picker.is_some() && scroll.is_some(),
            json!({}),
        );

        let (events_before, _) = desktop.received();
        let before_load = desktop.paints();
        desktop.send("Y").await;
        let flowing = eventually(WAIT, || desktop.received().0 > events_before).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (events, payload) = desktop.received();
        let (wire, at) = (link.delivered.load(Relaxed), Instant::now());
        let typed = type_during_load(&client, &terminal).await;
        tokio::time::sleep(Duration::from_secs(3).saturating_sub(at.elapsed())).await;
        let window = at.elapsed().as_secs_f64();
        let (events_after, payload_after) = desktop.received();
        let wire_after = link.delivered.load(Relaxed);
        let loaded_paint = before_load + ECHOES + 1;
        let loaded = desktop.painted(loaded_paint).await;
        let echo = echo_ms(&desktop.log.lock().unwrap().paint_log, before_load, &typed);
        j.guard(
            "the yes-like load streams for its window",
            flowing && loaded && events_after > events,
            json!({}),
        );

        let (watcher_client, _watcher_link) = linked(server, shape, RAW_DESKTOP).await;
        let watcher = RawDesktop::new(watcher_client, &terminal);
        let asked = Instant::now();
        let (watcher_stream, _) = watcher.attach(None).await;
        let replayed = watcher.painted(loaded_paint).await;
        let (first_output, full_screen) = {
            let log = watcher.log.lock().unwrap();
            (
                log.arrivals.first().map(|(at, _)| ms(*at - asked)),
                log.painted_at.map(|at| ms(at - asked)),
            )
        };
        let replay_bytes = watcher.received().1;
        watcher.leave(watcher_stream).await;
        j.guard(
            "a desktop attaching to it gets the screen",
            replayed && first_output.is_some(),
            json!({}),
        );

        let (ticks, started) = (cpu_ticks(pid), Instant::now());
        desktop.send("R").await;
        let repainted = desktop.painted(loaded_paint + REPAINTS).await;
        let (wall, cpu) = (started.elapsed(), cpu_ticks(pid) - ticks);
        j.guard(
            "the scripted repaint finishes",
            repainted,
            json!({ "paints": REPAINTS }),
        );

        tokio::time::sleep(Duration::from_secs(2)).await;
        let rss_before = rss_kib(pid);
        let mut idle = Vec::new();
        for _ in 0..IDLE_PTYS {
            let (id, _, events) = client
                .spawn_shell_terminal_stream(command.into(), 120, 40)
                .await
                .expect("spawn an idle stub");
            let pty = RawDesktop::new(client.clone(), &id);
            pty.watch(events);
            idle.push(pty);
        }
        let mut all_painted = true;
        for pty in &idle {
            all_painted &= pty.painted(1).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let rss_after = rss_kib(pid);
        j.guard(
            "20 idle PTYs paint and sit",
            all_painted,
            json!({ "ptys": IDLE_PTYS }),
        );
        for pty in idle {
            let _ = client.terminal_close(pty.terminal).await;
        }
        let _ = client.cancel_stream(stream_id).await;
        let _ = client.terminal_close(terminal).await;

        json!({
            "picker_redraw": picker,
            "scroll_60_lines": scroll,
            "yes_load": {
                "window_s": window,
                "events_per_s": (events_after - events) as f64 / window,
                "payload_bytes_per_s": (payload_after - payload) as f64 / window,
                "wire_bytes_per_s": (wire_after - wire) as f64 / window,
                "echo_ms": echo,
            },
            "attach": {
                "first_output_ms": first_output,
                "full_screen_ms": full_screen,
                "replay_payload_bytes": replay_bytes,
            },
            "scripted_repaint": {
                "paints": REPAINTS,
                "wall_ms": ms(wall),
                "server_cpu_ms": cpu * 1000 / USER_HZ,
                "server_cpu_pct": (cpu as f64 * 1000.0 / USER_HZ as f64 / wall.as_secs_f64()).round() / 10.0,
            },
            "idle_pty_rss": {
                "ptys": IDLE_PTYS,
                "before_kib": rss_before,
                "after_kib": rss_after,
                "per_pty_kib": (rss_after as f64 - rss_before as f64) / IDLE_PTYS as f64,
            },
        })
    }

    impl FrameDesktop {
        fn received(&self) -> (usize, usize) {
            let log = self.log.lock().unwrap();
            (log.frames, log.bytes)
        }
    }

    async fn frame_repaint(
        c: &FrameDesktop,
        link: &Link,
        observer: &RawDesktop,
        key: &str,
    ) -> Option<Value> {
        let ((frames, bytes), wire) = (c.received(), link.delivered.load(Relaxed));
        let paints = observer.paints();
        c.send(key).await;
        if !observer.painted(paints + 1).await {
            return None;
        }
        let mut last = (c.received(), Instant::now());
        while last.1.elapsed() < Duration::from_millis(500) {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if c.received() != last.0 {
                last = (c.received(), Instant::now());
            }
        }
        let (frames_after, bytes_after) = c.received();
        Some(json!({
            "payload_bytes": bytes_after - bytes,
            "wire_bytes": link.delivered.load(Relaxed) - wire,
            "frames": frames_after - frames,
        }))
    }

    async fn measure_frames(
        j: &mut Journey,
        server: &Server,
        command: &str,
        shape: &Shape,
    ) -> Value {
        let raw = server.connect_announcing(RAW_DESKTOP).await;
        let (terminal, stream_id, opened) = raw
            .spawn_shell_terminal_stream(command.into(), 120, 40)
            .await
            .expect("spawn the stub");
        let observer = RawDesktop::new(raw.clone(), &terminal);
        observer.watch(opened);
        observer.fit(120, 40).await;
        let opened = observer.painted(1).await;
        let (client, link) = linked(server, shape, Features::ALL).await;
        let c = FrameDesktop::new(client, &terminal);
        let (c_stream, _) = c.attach(None).await;
        c.fit(120, 40).await;
        let attached = eventually(WAIT, || c.received().0 > 0).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let picker = frame_repaint(&c, &link, &observer, "j").await;
        let scroll = frame_repaint(&c, &link, &observer, "S").await;
        j.guard(
            "a picker redraw and a 60-line scroll arrive as frames",
            opened && attached && picker.is_some() && scroll.is_some(),
            json!({}),
        );

        // As the raw run does: the yes-like load, then a second desktop attaches on its own link.
        let before_load = observer.paints();
        let (frames_before, _) = c.received();
        c.send("Y").await;
        let flowing = eventually(WAIT, || c.received().0 > frames_before).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let ((frames, payload), wire, at) =
            (c.received(), link.delivered.load(Relaxed), Instant::now());
        let typed = type_during_load(&c.client, &terminal).await;
        tokio::time::sleep(Duration::from_secs(3).saturating_sub(at.elapsed())).await;
        let window = at.elapsed().as_secs_f64();
        let ((frames_after, payload_after), wire_after) =
            (c.received(), link.delivered.load(Relaxed));
        let loaded_paint = before_load + ECHOES + 1;
        let loaded = observer.painted(loaded_paint).await && c.painted(loaded_paint).await;
        let (echo, echo_local, lag) = {
            let (far, near) = (c.log.lock().unwrap(), observer.log.lock().unwrap());
            let lag = reached(&far.paint_log, loaded_paint)
                .zip(reached(&near.paint_log, loaded_paint))
                .map(|(far, near)| ms(far.saturating_duration_since(near)));
            (
                echo_ms(&far.paint_log, before_load, &typed),
                echo_ms(&near.paint_log, before_load, &typed),
                lag,
            )
        };
        j.guard(
            "the yes-like load streams frames for its window",
            flowing && loaded && frames_after > frames,
            json!({}),
        );
        let (watcher_client, _watcher_link) = linked(server, shape, Features::ALL).await;
        let watcher = FrameDesktop::new(watcher_client, &terminal);
        let asked = Instant::now();
        let (watcher_stream, _) = watcher.attach(None).await;
        let first = eventually(WAIT, || watcher.log.lock().unwrap().first_frame.is_some()).await;
        let first_frame = watcher.log.lock().unwrap().first_frame;
        watcher.leave(watcher_stream).await;
        j.guard(
            "a desktop attaching after the load gets a frame",
            loaded && first,
            json!({}),
        );

        let pid = server.pid();
        let (ticks, started) = (cpu_ticks(pid), Instant::now());
        c.send("R").await;
        let repainted = observer.painted(loaded_paint + REPAINTS).await
            && c.painted(loaded_paint + REPAINTS).await;
        let (wall, cpu) = (started.elapsed(), cpu_ticks(pid) - ticks);
        j.guard(
            "the scripted repaint finishes on the frame client",
            repainted,
            json!({ "paints": REPAINTS }),
        );

        c.leave(c_stream).await;
        let _ = raw.cancel_stream(stream_id).await;
        let _ = raw.terminal_close(terminal).await;
        json!({
            "picker_redraw": picker,
            "scroll_60_lines": scroll,
            "yes_load": {
                "window_s": window,
                "frames_per_s": (frames_after - frames) as f64 / window,
                "payload_bytes_per_s": (payload_after - payload) as f64 / window,
                "wire_bytes_per_s": (wire_after - wire) as f64 / window,
                "echo_ms": echo,
                "echo_local_ms": echo_local,
                "last_paint_lag_ms": lag,
            },
            "attach": {
                "first_frame_ms": first_frame.map(|(at, _)| ms(at - asked)),
                "first_frame_bytes": first_frame.map(|(_, bytes)| bytes),
            },
            "scripted_repaint": {
                "paints": REPAINTS,
                "wall_ms": ms(wall),
                "server_cpu_ms": cpu * 1000 / USER_HZ,
                "server_cpu_pct": (cpu as f64 * 1000.0 / USER_HZ as f64 / wall.as_secs_f64()).round() / 10.0,
            },
            "shell_history": shell_history(server, shape).await,
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "baseline numbers: run alone, in release, 5 times; see the screen-model baseline notes"]
    async fn shared_terminal_screen_baseline() {
        let sandbox = Sandbox::new();
        let mut j = Journey::new("shared_terminal_screen_baseline", &sandbox);
        let server = Server::start(&sandbox).await;
        let command = stub_command(&sandbox);
        let mut numbers = serde_json::Map::new();
        for shape in &SHAPES {
            j.step(&format!("raw bytes on a {} link", shape.name));
            let measured = measure(&mut j, &server, &command, shape).await;
            numbers.insert(shape.name.to_string(), measured);
            j.step(&format!("screen frames on a {} link", shape.name));
            let measured = measure_frames(&mut j, &server, &command, shape).await;
            numbers.insert(format!("{}, frames", shape.name), measured);
        }
        j.note("baseline", Value::Object(numbers));
        j.finish();
    }
}
