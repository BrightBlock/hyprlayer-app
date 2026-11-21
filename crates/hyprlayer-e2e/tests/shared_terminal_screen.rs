//! Two desktops, A at 120x40 dark and B at 80x24 light, render one shared terminal from the raw
//! bytes they receive. The golden records each case's outcome as it is today, failures included.
//! Desktops C and D render the server's screen instead, as frames written into xterm 5.5
//! (`xterm/render.mjs`, which needs `node` and the repository's `pnpm install`); each must show
//! what the server's libghostty screen holds, cells and history, after every case.
#![cfg(unix)]

use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ghostty_vt::{CellColor, CellWide, RowAt, Terminal};
use hyprlayer_core::models::{Rgb, TerminalTheme};
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_e2e::agent_stub::{install, SCREEN_TUI};
use hyprlayer_e2e::link::{far_phone, far_phone_announcing};
use hyprlayer_e2e::{json, keystrokes, Journey, Sandbox, Server, Value};
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

#[derive(Clone, Copy)]
struct Theme {
    background: &'static str,
    scheme: &'static str,
}

const DARK: Theme = Theme {
    background: "\x1b]11;rgb:0000/0000/0000\x1b\\",
    scheme: "\x1b[?997;1n",
};
const LIGHT: Theme = Theme {
    background: "\x1b]11;rgb:ffff/ffff/ffff\x1b\\",
    scheme: "\x1b[?997;2n",
};
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

enum Seen {
    Resized(u16, u16),
    Output(Vec<u8>),
    Reset,
}

struct Log {
    render: bool,
    seen: Vec<Seen>,
    parser: vt100::Parser,
    stream: u64,
    next: Option<u64>,
    gaps: usize,
    paints: u64,
    painted_at: Option<Instant>,
    arrivals: Vec<(Instant, usize)>,
    tail: Vec<u8>,
    model: Option<Model>,
}

impl Log {
    fn scan(&mut self, bytes: &[u8], theme: Theme) -> Vec<&'static str> {
        let mut buf = std::mem::take(&mut self.tail);
        let old = buf.len();
        buf.extend_from_slice(bytes);
        let mut replies = Vec::new();
        if buf.contains(&b'?') {
            for (query, reply) in [
                (&b"\x1b]11;?\x07"[..], theme.background),
                (b"\x1b]11;?\x1b\\", theme.background),
                (b"\x1b[?996n", theme.scheme),
            ] {
                replies.extend(
                    find(&buf, query)
                        .filter(|at| at + query.len() > old)
                        .map(|_| reply),
                );
            }
        }
        if buf.contains(&b'(') {
            for at in find(&buf, b"(paint ") {
                let digits = &buf[at + 7..];
                let len = digits.iter().take_while(|c| c.is_ascii_digit()).count();
                if digits.get(len) == Some(&b')') && at + 7 + len >= old {
                    let paint = std::str::from_utf8(&digits[..len])
                        .ok()
                        .and_then(|d| d.parse::<u64>().ok());
                    if let Some(paint) = paint.filter(|&paint| paint > self.paints) {
                        self.paints = paint;
                        self.painted_at = Some(Instant::now());
                    }
                }
            }
        }
        self.tail = buf[buf.len().saturating_sub(16)..].to_vec();
        replies
    }
}

fn find<'a>(hay: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    hay.windows(needle.len())
        .enumerate()
        .filter(move |(_, window)| *window == needle)
        .map(|(at, _)| at)
}

// Mirrors TerminalFeed and xterm: a resume whose first output isn't where it asked starts over.
#[derive(Clone)]
struct Desktop {
    client: Arc<RpcClient>,
    terminal: String,
    theme: Theme,
    log: Arc<Mutex<Log>>,
}

impl Desktop {
    fn new(
        client: Arc<RpcClient>,
        terminal: &str,
        theme: Theme,
        fit: (u16, u16),
        render: bool,
    ) -> Self {
        Self {
            client,
            terminal: terminal.to_string(),
            theme,
            log: Arc::new(Mutex::new(Log {
                render,
                seen: Vec::new(),
                parser: vt100::Parser::new(fit.1, fit.0, 0),
                stream: 0,
                next: None,
                gaps: 0,
                paints: 0,
                painted_at: None,
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

    fn watch(&self, mut stream: TerminalStream, from: Option<u64>) {
        let me = self.clone();
        let generation = {
            let mut log = self.log.lock().unwrap();
            log.stream += 1;
            log.stream
        };
        tokio::spawn(async move {
            let mut first = true;
            while let Some(event) = stream.recv().await {
                let mut log = me.log.lock().unwrap();
                if log.stream != generation {
                    return;
                }
                match event {
                    TerminalEvent::Resized { cols, rows } => {
                        if let Some(model) = &log.model {
                            model.resize(cols, rows);
                        }
                        if log.render {
                            log.seen.push(Seen::Resized(cols, rows));
                        }
                    }
                    TerminalEvent::Clipboard { .. } => {}
                    TerminalEvent::Output { data, offset, .. } => {
                        let bytes = STANDARD.decode(data).expect("output is base64");
                        if first && from.is_some() && offset != from {
                            log.seen.push(Seen::Reset);
                        } else if matches!((offset, log.next), (Some(at), Some(next)) if at > next)
                        {
                            log.gaps += 1;
                        }
                        first = false;
                        log.next = offset.map(|at| at + bytes.len() as u64);
                        log.arrivals.push((Instant::now(), bytes.len()));
                        if let Some(model) = &log.model {
                            model.feed(&bytes);
                        }
                        let replies = log.scan(&bytes, me.theme);
                        if log.render {
                            log.seen.push(Seen::Output(bytes));
                        }
                        drop(log);
                        for reply in replies {
                            let (client, terminal) = (me.client.clone(), me.terminal.clone());
                            tokio::spawn(async move {
                                let _ = client.terminal_write(terminal, keystrokes(reply)).await;
                            });
                        }
                    }
                    TerminalEvent::Exited { .. } => return,
                }
            }
        });
    }

    fn forget_stream(&self) {
        self.log.lock().unwrap().stream += 1;
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

    async fn attach(&self, from: Option<u64>) -> (u64, bool) {
        let attached = self
            .client
            .terminal_attach_stream(self.terminal.clone(), from)
            .await
            .expect("terminal_attach")
            .expect("a live terminal");
        self.watch(attached.events, from);
        (attached.stream_id, attached.resumed)
    }

    async fn painted(&self, paint: u64) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.log.lock().unwrap().paints >= paint {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
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

    fn next(&self) -> Option<u64> {
        self.log.lock().unwrap().next
    }

    fn screen(&self) -> vt100::Screen {
        let mut guard = self.log.lock().unwrap();
        let log = &mut *guard;
        for seen in std::mem::take(&mut log.seen) {
            match seen {
                Seen::Resized(cols, rows) => log.parser.screen_mut().set_size(rows, cols),
                Seen::Output(bytes) => log.parser.process(&bytes),
                Seen::Reset => {
                    let (rows, cols) = log.parser.screen().size();
                    log.parser = vt100::Parser::new(rows, cols, 0);
                }
            }
        }
        log.parser.screen().clone()
    }
}

/// The server's screen: the same libghostty, fed the same bytes at the same sizes as desktop A
/// is sent, with the server's scrollback limit.
#[derive(Clone)]
struct Model(Arc<Mutex<Terminal>>);

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
                let style = &cell.style;
                // xterm draws a linked cell underlined.
                let link = cell.hyperlink.is_some();
                let flags = [
                    style.bold,
                    style.faint,
                    style.italic,
                    style.underlined || link,
                    style.inverse,
                    style.strikethrough,
                    style.invisible,
                    style.blink,
                ]
                .iter()
                .enumerate()
                .filter(|(_, on)| **on)
                .fold(0, |flags, (bit, _)| flags | 1 << bit);
                cells.push(ModelCell {
                    text: cell
                        .graphemes
                        .iter()
                        .filter_map(|&c| char::from_u32(c))
                        .collect(),
                    width: match cell.wide {
                        CellWide::SpacerTail => 0,
                        CellWide::Wide => 2,
                        _ => 1,
                    },
                    fg: color(style.fg_color),
                    bg: color(style.bg_color),
                    flags,
                });
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

#[derive(Clone)]
struct Xterm(mpsc::UnboundedSender<(String, Option<oneshot::Sender<Value>>)>);

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
        Self(tx)
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
}

#[derive(Clone)]
struct FrameDesktop {
    client: Arc<RpcClient>,
    terminal: String,
    xterm: Xterm,
    log: Arc<Mutex<FrameLog>>,
}

impl FrameDesktop {
    fn new(client: Arc<RpcClient>, terminal: &str) -> Self {
        Self {
            client,
            terminal: terminal.to_string(),
            xterm: Xterm::spawn(),
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
        let generation = {
            let mut log = self.log.lock().unwrap();
            log.stream += 1;
            log.stream
        };
        let (me, mut stream) = (self.clone(), attached.events);
        tokio::spawn(async move {
            while let Some(event) = stream.recv().await {
                let mut log = me.log.lock().unwrap();
                if log.stream != generation {
                    return;
                }
                match event {
                    TerminalEvent::Resized { cols, rows } => {
                        me.xterm.send(json!({ "resize": [cols, rows] }))
                    }
                    TerminalEvent::Output {
                        data,
                        offset,
                        frame: Some(_),
                    } => {
                        log.frames += 1;
                        log.bytes += STANDARD.decode(&data).map_or(0, |bytes| bytes.len());
                        me.xterm.send(json!({ "write": data }));
                        log.history = offset;
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
        (attached.stream_id, attached.resumed)
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

    fn copies(&self) -> Vec<String> {
        self.log.lock().unwrap().copies.clone()
    }

    async fn shows(&self, model: &Model) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (snapshot, view) = (model.snapshot(), self.xterm.dump().await);
            let bad = mismatched(&snapshot, &view);
            let history = history_matches(&snapshot, &view);
            if (bad == 0 && history) || Instant::now() >= deadline {
                let raw = self.log.lock().unwrap().raw;
                return json!({
                    "grids_match": bad == 0,
                    "mismatched_cells": bad,
                    "history_matches": history,
                    "raw_output_events": raw,
                });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

const SHOWS: &str = "the frame client shows the server's screen, cells and history";

fn frames_ok(evidence: &Value) -> bool {
    evidence["grids_match"] == json!(true)
        && evidence["history_matches"] == json!(true)
        && evidence["raw_output_events"] == json!(0)
}

fn compare(a: &Desktop, b: &Desktop, answers_seen: u64) -> Value {
    let (a, b) = (a.screen(), b.screen());
    let ((a_rows, a_cols), (b_rows, b_cols)) = (a.size(), b.size());
    let mismatched = (0..a_rows.max(b_rows))
        .flat_map(|row| (0..a_cols.max(b_cols)).map(move |col| (row, col)))
        .filter(|&(row, col)| a.cell(row, col) != b.cell(row, col))
        .count();
    json!({
        "grids_match": mismatched == 0,
        "mismatched_cells": mismatched,
        "answers_seen": answers_seen,
    })
}

fn reported(desktop: &Desktop, label: &str) -> Value {
    let contents = desktop.screen().contents();
    // A soft-wrapped row above can join the label's row in `contents`.
    let line = contents
        .lines()
        .find_map(|line| line.find(label).map(|at| &line[at..]))
        .unwrap_or_default();
    let counts: serde_json::Map<String, Value> = line
        .split_whitespace()
        .filter_map(|pair| pair.split_once('='))
        .filter_map(|(key, n)| Some((key.to_string(), json!(n.parse::<u64>().ok()?))))
        .collect();
    Value::Object(counts)
}

fn show(j: &mut Journey, case: &str, a: &Desktop, b: &Desktop) {
    let rows = |d: &Desktop| {
        d.screen()
            .contents()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    j.note(
        &format!("screens {case}"),
        json!({ "a": rows(a), "b": rows(b) }),
    );
}

fn with(mut evidence: Value, extra: Value) -> Value {
    if let (Some(evidence), Value::Object(extra)) = (evidence.as_object_mut(), extra) {
        evidence.extend(extra);
    }
    evidence
}

const DREW: &str = "A saw every byte and both desktops drew the repaint";

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

async fn drawn(a: &Desktop, b: &Desktop, paint: u64) -> bool {
    a.painted(paint).await && b.painted(paint).await && a.gaps() == 0
}

fn stub_command(sandbox: &Sandbox) -> String {
    let stub = install(sandbox, "screen-tui", SCREEN_TUI);
    format!("'{}' '{FIXTURE}'", stub.display())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_terminal_screen() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("shared_terminal_screen", &sandbox);
    let server = Server::start(&sandbox).await;
    let (b_client, b_link) = far_phone(server.socket(), true, Duration::ZERO, false).await;

    j.step("A opens the program at 120x40 and B attaches at 80x24");
    let (terminal, a_stream, opened) = server
        .client()
        .spawn_shell_terminal_stream(stub_command(&sandbox), 120, 40)
        .await
        .expect("spawn the stub");
    let model = Model::new(120, 40);
    let a = Desktop::new(server.client(), &terminal, DARK, (120, 40), true).modelled(model.clone());
    a.watch(opened, None);
    a.fit(120, 40).await;
    let opened = a.painted(1).await;
    let b = Desktop::new(b_client, &terminal, LIGHT, (80, 24), true);
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
    let ok = opened && drawn(&a, &b, 1).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "attach", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("(a) A resizes");
    a.fit(100, 30).await;
    let ok = drawn(&a, &b, 2).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "(a)", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("(b) B types, which gives it the size, then resizes");
    b.send("j").await;
    let typed = drawn(&a, &b, 4).await;
    b.fit(90, 28).await;
    let ok = typed && drawn(&a, &b, 5).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "(b)", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("(c) B answers the picker, which erases and redraws it");
    b.send("\r").await;
    let ok = drawn(&a, &b, 6).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "(c)", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("(d) the program asks for the theme while B, the light desktop, is active");
    b.send("t").await;
    let ok = drawn(&a, &b, 7).await;
    let answers = reported(&a, "ANSWERS");
    let osc11 = answers["osc11"].as_u64().unwrap_or_default();
    j.guard(
        DREW,
        ok,
        with(
            compare(&a, &b, osc11),
            json!({
                "scheme_answers": answers["scheme"],
                "themes_heard": { "dark": answers["dark"], "light": answers["light"] },
                "active": "light",
            }),
        ),
    );
    show(&mut j, "(d)", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("Ctrl+F3 and a key batched with a reply reach the program");
    b.send("h").await;
    let listening = drawn(&a, &b, 8).await;
    b.send("\x1b[1;5R").await;
    b.send(&format!("x{}", LIGHT.background)).await;
    let ok = listening && drawn(&a, &b, 9).await;
    let heard = reported(&a, "HEARD");
    j.guard(
        DREW,
        ok,
        with(
            compare(&a, &b, 0),
            json!({ "ctrl_f3_heard": heard["ctrl_f3"], "batched_key_heard": heard["batched_key"] }),
        ),
    );
    show(&mut j, "ctrl+f3", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("(e) B's link stalls for 5 s while the program writes, then B reads");
    b_link.down_stalled.store(true, Relaxed);
    let stalled_at = Instant::now();
    let typing = {
        let b = b.clone();
        tokio::spawn(async move { b.send("F").await })
    };
    let a_drew = a.painted(10).await;
    tokio::time::sleep_until((stalled_at + STALL).into()).await;
    b_link.down_stalled.store(false, Relaxed);
    let _ = typing.await;
    let ok = a_drew && drawn(&a, &b, 10).await;
    j.guard(
        DREW,
        ok,
        with(
            compare(&a, &b, 0),
            json!({ "b_skipped_output": b.gaps() > 0 }),
        ),
    );
    show(&mut j, "(e)", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("(f) B reconnects after missing more output than the terminal keeps");
    let from = b.next();
    b.client
        .cancel_stream(b_stream)
        .await
        .expect("B's stream drops");
    b.forget_stream();
    let size_back = a.painted(11).await;
    a.send("F").await;
    let flooded = a.painted(12).await;
    let (stream, resumed) = b.attach(from).await;
    b_stream = stream;
    b.fit(80, 24).await;
    let ok = size_back && flooded && drawn(&a, &b, 12).await;
    j.guard(
        DREW,
        ok,
        with(compare(&a, &b, 0), json!({ "resumed": resumed })),
    );
    show(&mut j, "(f)", &a, &b);
    frames_shown(&mut j, &c, &model).await;

    j.step("frames: C draws a row of emoji, CJK and a flag on a coloured picker row");
    let paints = a.paints();
    c.fit(110, 32).await;
    c.send("E").await;
    let settled = a.repainted(paints).await;
    let evidence = c.shows(&model).await;
    j.guard(SHOWS, settled && frames_ok(&evidence), evidence);

    j.step("frames: a URL the program wrote past the right margin stays one wrapped line");
    let paints = a.paints();
    c.send("U").await;
    let settled = a.repainted(paints).await;
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
    let paints = a.paints();
    c_link.down_stalled.store(true, Relaxed);
    // Its answer can't come back until the link does.
    let typing = {
        let c = c.clone();
        tokio::spawn(async move { c.send("L").await })
    };
    let settled = a.repainted(paints).await;
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
        let paints = a.paints();
        c.fit(cols, rows).await;
        let settled = a.repainted(paints).await;
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
    let paints = a.paints();
    c.send("j").await;
    let drew = a.repainted(paints).await;
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
    c.fit(124, 40).await;
    let paints = a.paints();
    c.send("C").await;
    let settled = a.repainted(paints).await;
    let owner = c
        .copies()
        .iter()
        .any(|copy| copy == "copied by the program");
    let other = !d.copies().is_empty();
    let (c_shows, d_shows) = (c.shows(&model).await, d.shows(&model).await);
    j.guard(
        "only the size owner's frame stream carries the copy",
        settled && owner && !other && frames_ok(&c_shows) && frames_ok(&d_shows),
        json!({ "owner_copied": owner, "other_copied": other, "c": c_shows, "d": d_shows }),
    );

    j.step("frames: focus flips between C at 124 and D at 72 columns, the program redrawing everything each time");
    let paints = a.paints();
    c.send("W").await;
    let mut redrawn = a.repainted(paints).await;
    let flips = 20;
    for flip in 0..flips {
        if flip % 2 == 0 {
            d.focus().await
        } else {
            c.focus().await
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    redrawn &= a.settled(Duration::from_millis(1500)).await;
    let (c_shows, d_shows) = (c.shows(&model).await, d.shows(&model).await);
    let paints = a.paints();
    c.send("w").await;
    redrawn &= a.repainted(paints).await;
    j.guard(
        "both frame desktops show the server's screen and history after the flips",
        redrawn && frames_ok(&c_shows) && frames_ok(&d_shows),
        json!({ "flips": flips, "c": c_shows, "d": d_shows }),
    );

    j.step("(g) with nobody watching, the program asks for the theme");
    a.client
        .terminal_theme(A_THEME)
        .await
        .expect("A reports its theme");
    for (desktop, stream) in [(&a, a_stream), (&b, b_stream)] {
        let _ = desktop.client.cancel_stream(stream).await;
        desktop.forget_stream();
    }
    c.leave(c_stream).await;
    d.leave(d_stream).await;
    let paints = a.paints();
    tokio::time::sleep(Duration::from_secs(1)).await;
    a.send("t").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (a_stream, _) = a.attach(None).await;
    let painted = a.painted(paints + 1).await;
    let answers = reported(&a, "ANSWERS");
    j.guard(
        "the server answers each query once, in the theme of the desktop that last owned the size",
        painted
            && answers["osc11"] == json!(1)
            && answers["scheme"] == json!(1)
            && answers["dark"] == json!(1)
            && answers["light"] == json!(0),
        json!({ "answers": answers }),
    );

    let _ = a.client.cancel_stream(a_stream).await;
    let _ = server.client().terminal_close(terminal).await;
    j.finish();
}

#[cfg(target_os = "linux")]
mod baseline {
    use super::*;
    use hyprlayer_e2e::eventually;
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

    async fn linked(server: &Server, shape: &Shape) -> (Arc<RpcClient>, Arc<Link>) {
        let (client, link) = far_phone(server.socket(), true, shape.one_way, false).await;
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

    impl Desktop {
        fn received(&self) -> (usize, usize) {
            let log = self.log.lock().unwrap();
            (
                log.arrivals.len(),
                log.arrivals.iter().map(|(_, n)| n).sum(),
            )
        }
    }

    async fn repaint_bytes(desktop: &Desktop, link: &Link, key: &str, paint: u64) -> Option<Value> {
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
        let (client, link) = linked(server, shape).await;
        let (terminal, stream_id, opened) = client
            .spawn_shell_terminal_stream(command.into(), 120, 40)
            .await
            .expect("spawn the stub");
        let desktop = Desktop::new(client.clone(), &terminal, DARK, (120, 40), false);
        desktop.watch(opened, None);
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
        desktop.send("Y").await;
        let flowing = eventually(WAIT, || desktop.received().0 > events_before).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (events, payload) = desktop.received();
        let (wire, at) = (link.delivered.load(Relaxed), Instant::now());
        tokio::time::sleep(Duration::from_secs(3)).await;
        let window = at.elapsed().as_secs_f64();
        let (events_after, payload_after) = desktop.received();
        let wire_after = link.delivered.load(Relaxed);
        let loaded = desktop.painted(4).await;
        j.guard(
            "the yes-like load streams for its window",
            flowing && loaded && events_after > events,
            json!({}),
        );

        let (watcher_client, _watcher_link) = linked(server, shape).await;
        let watcher = Desktop::new(watcher_client, &terminal, LIGHT, (80, 24), false);
        let asked = Instant::now();
        let (watcher_stream, _) = watcher.attach(None).await;
        let replayed = watcher.painted(4).await;
        let (first_output, full_screen) = {
            let log = watcher.log.lock().unwrap();
            (
                log.arrivals.first().map(|(at, _)| ms(*at - asked)),
                log.painted_at.map(|at| ms(at - asked)),
            )
        };
        let replay_bytes = watcher.received().1;
        let _ = watcher.client.cancel_stream(watcher_stream).await;
        watcher.forget_stream();
        j.guard(
            "a desktop attaching to it gets the screen",
            replayed && first_output.is_some(),
            json!({}),
        );

        let (ticks, started) = (cpu_ticks(pid), Instant::now());
        desktop.send("R").await;
        let repainted = desktop.painted(4 + REPAINTS).await;
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
            let pty = Desktop::new(client.clone(), &id, DARK, (120, 40), false);
            pty.watch(events, None);
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
        observer: &Desktop,
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
        let (terminal, stream_id, opened) = server
            .client()
            .spawn_shell_terminal_stream(command.into(), 120, 40)
            .await
            .expect("spawn the stub");
        let observer = Desktop::new(server.client(), &terminal, DARK, (120, 40), false);
        observer.watch(opened, None);
        observer.fit(120, 40).await;
        let opened = observer.painted(1).await;
        let (client, link) = far_phone_announcing(
            server.socket(),
            true,
            shape.one_way,
            false,
            Some(Features::ALL),
        )
        .await;
        link.down_bps.store(shape.bps, Relaxed);
        link.up_bps.store(shape.bps, Relaxed);
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
        let _ = c.client.cancel_stream(c_stream).await;
        let _ = server.client().cancel_stream(stream_id).await;
        let _ = server.client().terminal_close(terminal).await;
        json!({ "picker_redraw": picker, "scroll_60_lines": scroll })
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
