//! Two desktops, A at 120x40 dark and B at 80x24 light, render one shared terminal from the raw
//! bytes they receive. The golden records each case's outcome as it is today, failures included.
#![cfg(unix)]

use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_e2e::agent_stub::{install, SCREEN_TUI};
use hyprlayer_e2e::link::far_phone;
use hyprlayer_e2e::{json, keystrokes, Journey, Sandbox, Server, Value};
use hyprlayer_server::client::{RpcClient, TerminalStream};

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
            })),
        }
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
                        if log.render {
                            log.seen.push(Seen::Resized(cols, rows));
                        }
                    }
                    TerminalEvent::Output { data, offset } => {
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
    let line = contents
        .lines()
        .find(|line| line.trim_start().starts_with(label))
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
    let (terminal, _, opened) = server
        .client()
        .spawn_shell_terminal_stream(stub_command(&sandbox), 120, 40)
        .await
        .expect("spawn the stub");
    let a = Desktop::new(server.client(), &terminal, DARK, (120, 40), true);
    a.watch(opened, None);
    a.fit(120, 40).await;
    let opened = a.painted(1).await;
    let b = Desktop::new(b_client, &terminal, LIGHT, (80, 24), true);
    let (mut b_stream, _) = b.attach(None).await;
    b.fit(80, 24).await;
    let ok = opened && drawn(&a, &b, 1).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "attach", &a, &b);

    j.step("(a) A resizes");
    a.fit(100, 30).await;
    let ok = drawn(&a, &b, 2).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "(a)", &a, &b);

    j.step("(b) B types, which gives it the size, then resizes");
    b.send("j").await;
    let typed = drawn(&a, &b, 4).await;
    b.fit(90, 28).await;
    let ok = typed && drawn(&a, &b, 5).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "(b)", &a, &b);

    j.step("(c) B answers the picker, which erases and redraws it");
    b.send("\r").await;
    let ok = drawn(&a, &b, 6).await;
    j.guard(DREW, ok, compare(&a, &b, 0));
    show(&mut j, "(c)", &a, &b);

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

    let _ = b.client.cancel_stream(b_stream).await;
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
        }
        j.note("baseline", Value::Object(numbers));
        j.finish();
    }
}
