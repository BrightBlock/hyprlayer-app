//! Two desktops on one terminal agree on its size: each renders the output at the
//! size it was written for, a watching desktop's own fit leaves the terminal alone,
//! and when the active desktop leaves, the size goes to the one still watching.
#![cfg(unix)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hyprlayer_e2e::{eventually, json, keystrokes, Journey, Sandbox, Server, Value};
use hyprlayer_server::client::TerminalStream;
use tokio::time::Instant;

const WAIT: Duration = Duration::from_secs(10);
/// Long enough for the trap below to have printed a resize that happened.
const SETTLE: Duration = Duration::from_millis(1000);
/// Prints the pty's size on every resize, and on each line typed draws a picker
/// with a 105-column line, then erases it two rows up and redraws, as a TUI does.
const SHELL: &str = "trap 'echo \"SIZE $(stty size)\"' WINCH; \
    (while read -r line; do printf 'PICK %s\\nfooter\\n' \"$(printf '%0100d' 0)\"; \
    printf '\\033[2A\\033[JPICKED\\n'; done) </dev/tty & \
    echo READY; while :; do sleep 0.05; done";

/// What xterm answers a program's queries with: status, cursor, DA1/DA2, DECRQM, colours, focus.
const REPLIES: &[&str] = &[
    "\x1b[?997;1n",
    "\x1b[0n",
    "\x1b[12;40R",
    "\x1b[?62;22c",
    "\x1b[>0;276;0c",
    "\x1b[?2026;2$y",
    "\x1b[4;2$y",
    "\x1b]11;rgb:0000/0000/0000\x1b\\",
    "\x1b]10;rgb:ffff/ffff/ffff\x07",
    "\x1b]4;1;rgb:cdcd/0000/0000\x1b\\",
    "\x1b[I",
    "\x1b[O",
];

/// One stream's events in order, outputs as text and offsets.
#[derive(Clone, Default)]
struct Timeline(Arc<Mutex<Vec<Value>>>);

impl Timeline {
    fn record(mut stream: TerminalStream) -> Self {
        let timeline = Self::default();
        let events = timeline.0.clone();
        tokio::spawn(async move {
            while let Some(event) = stream.recv().await {
                let mut value = serde_json::to_value(&event).expect("an event serializes");
                if let Some(data) = value["data"].as_str() {
                    let bytes = STANDARD.decode(data).expect("output is base64");
                    let end = value["offset"].as_u64().map(|at| at + bytes.len() as u64);
                    value = json!({
                        "kind": "output",
                        "text": String::from_utf8_lossy(&bytes),
                        "data": data,
                        "end": end,
                    });
                }
                events.lock().unwrap().push(value);
            }
        });
        timeline
    }

    fn events(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    fn text(&self) -> String {
        self.events()
            .iter()
            .filter_map(|e| e["text"].as_str().map(str::to_string))
            .collect()
    }

    /// The size this stream had told its client when the output holding `needle` arrived.
    fn size_at(&self, needle: &str) -> Option<[u64; 2]> {
        let mut size = None;
        let mut text = String::new();
        for event in self.events() {
            match event["kind"].as_str() {
                Some("resized") => {
                    size = Some([
                        event["cols"].as_u64().unwrap_or_default(),
                        event["rows"].as_u64().unwrap_or_default(),
                    ])
                }
                Some("output") => {
                    text.push_str(event["text"].as_str().unwrap_or_default());
                    if text.contains(needle) {
                        return size;
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The screen a desktop shows: fitted to its own `[cols, rows]` until the stream gives it a size.
    fn screen(&self, fit: [u16; 2]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(fit[1], fit[0], 0);
        for event in self.events() {
            match event["kind"].as_str() {
                Some("resized") => parser.screen_mut().set_size(
                    event["rows"].as_u64().unwrap_or_default() as u16,
                    event["cols"].as_u64().unwrap_or_default() as u16,
                ),
                Some("output") => parser.process(
                    &STANDARD
                        .decode(event["data"].as_str().unwrap_or_default())
                        .unwrap_or_default(),
                ),
                _ => {}
            }
        }
        parser
    }

    /// Every size this stream told its client, in order.
    fn sizes(&self) -> Vec<[u64; 2]> {
        self.events()
            .iter()
            .filter(|e| e["kind"] == json!("resized"))
            .map(|e| {
                [
                    e["cols"].as_u64().unwrap_or_default(),
                    e["rows"].as_u64().unwrap_or_default(),
                ]
            })
            .collect()
    }

    /// Where the output this stream has delivered ends.
    fn end(&self) -> Option<u64> {
        self.events().iter().rev().find_map(|e| e["end"].as_u64())
    }

    async fn wait_for(&self, needle: &str) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    async fn wait_for_size(&self, size: [u64; 2]) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.sizes().last() == Some(&size) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_terminal_size() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("shared_terminal_size", &sandbox);
    let server = Server::start(&sandbox).await;
    let wide = server.client();
    let narrow = server.connect().await;

    j.step("two desktops attach at different sizes");
    let (terminal, _, opened) = wide
        .spawn_shell_terminal_stream(SHELL.into(), 120, 40)
        .await
        .expect("spawn the terminal");
    let a = Timeline::record(opened);
    let ready = a.wait_for("READY").await;
    wide.terminal_resize(terminal.clone(), 120, 40)
        .await
        .expect("the opener reports its fit");
    let attached = narrow
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let b_stream = attached.stream_id;
    let b = Timeline::record(attached.events);
    narrow
        .terminal_resize(terminal.clone(), 80, 24)
        .await
        .expect("the watcher reports its fit");
    tokio::time::sleep(SETTLE).await;
    j.guard(
        "both desktops learn the terminal's one size",
        ready && a.sizes().last() == Some(&[120, 40]) && b.sizes().last() == Some(&[120, 40]),
        json!({ "size": [120, 40] }),
    );
    j.guard(
        "a watching desktop's fit leaves the terminal's size alone",
        ready && !a.text().contains("SIZE"),
        json!({ "printed": a.text().contains("SIZE") }),
    );

    j.step("a watching desktop remounts and its terminal answers the program's queries");
    narrow
        .cancel_stream(b_stream)
        .await
        .expect("unmount the watcher");
    let remounted = narrow
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let b_stream = remounted.stream_id;
    let b = Timeline::record(remounted.events);
    narrow
        .terminal_resize(terminal.clone(), 80, 24)
        .await
        .expect("the watcher reports its fit");
    for reply in REPLIES {
        narrow
            .terminal_write(terminal.clone(), keystrokes(reply))
            .await
            .expect("answer a query");
    }
    narrow
        .terminal_write(terminal.clone(), keystrokes(&REPLIES.concat()))
        .await
        .expect("answer every query in one write");
    tokio::time::sleep(SETTLE).await;
    j.guard(
        "its replies leave the terminal's size alone",
        !a.text().contains("SIZE") && b.sizes().last() == Some(&[120, 40]),
        json!({ "replies": REPLIES.len() }),
    );

    j.step("a multi-line redraw with no resize renders the same on both");
    wide.terminal_write(terminal.clone(), keystrokes("\r"))
        .await
        .expect("answer the picker");
    let picked = a.wait_for("PICKED").await && b.wait_for("PICKED").await;
    let (wide_screen, narrow_screen) = (a.screen([120, 40]), b.screen([80, 24]));
    let (wide_rows, narrow_rows) = (
        wide_screen.screen().contents(),
        narrow_screen.screen().contents(),
    );
    j.guard(
        "both desktops show the same grid",
        picked
            && wide_screen.screen().size() == narrow_screen.screen().size()
            && wide_rows == narrow_rows,
        json!({ "picked": picked }),
    );
    j.guard(
        "and it is the redraw the program meant",
        picked && wide_rows.contains("PICKED") && !narrow_rows.contains("PICK 0"),
        json!({}),
    );

    j.step("the desktop that types takes the size");
    let before_typing = a.end();
    narrow
        .terminal_write(terminal.clone(), keystrokes(" "))
        .await
        .expect("type into the terminal");
    let redrawn = a.wait_for("SIZE 24 80").await && b.wait_for("SIZE 24 80").await;
    j.guard(
        "the terminal takes the typist's fit",
        redrawn,
        json!({ "printed": "SIZE 24 80" }),
    );
    j.guard(
        "each desktop renders what follows at that size",
        a.size_at("SIZE 24 80") == Some([80, 24]) && b.size_at("SIZE 24 80") == Some([80, 24]),
        json!({ "size": [80, 24] }),
    );

    j.step("a resize from the active desktop reaches the watching one first");
    narrow
        .terminal_resize(terminal.clone(), 100, 30)
        .await
        .expect("the active desktop resizes");
    let resized = a.wait_for("SIZE 30 100").await;
    j.guard(
        "the watcher switches size before the output written for it",
        resized && a.size_at("SIZE 30 100") == Some([100, 30]),
        json!({ "size": [100, 30] }),
    );

    j.step("the active desktop leaves and the size goes back");
    narrow
        .cancel_stream(b_stream)
        .await
        .expect("detach the active desktop");
    let reconverged = a.wait_for("SIZE 40 120").await && a.wait_for_size([120, 40]).await;
    j.guard(
        "the remaining desktop's fit takes over",
        reconverged && a.size_at("SIZE 40 120") == Some([120, 40]),
        json!({ "size": [120, 40] }),
    );

    j.step("a resume replays the sizes in line with the output");
    let resumed = wide
        .terminal_attach_stream(terminal.clone(), before_typing)
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let replay = Timeline::record(resumed.events);
    let replayed = replay.wait_for("SIZE 40 120").await;
    j.guard(
        "a resume learns the size before any output",
        before_typing.is_some()
            && resumed.resumed
            && replayed
            && replay.events().first().map(|e| e["kind"].clone()) == Some(json!("resized")),
        json!({}),
    );
    j.guard(
        "and renders each stretch at the size it was written for",
        replay.size_at("SIZE 24 80") == Some([80, 24])
            && replay.size_at("SIZE 30 100") == Some([100, 30])
            && replay.size_at("SIZE 40 120") == Some([120, 40]),
        json!({ "sizes": [[80, 24], [100, 30], [120, 40]] }),
    );
    let _ = wide.cancel_stream(resumed.stream_id).await;

    j.step("focusing a desktop gives it the size");
    let back = narrow
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach")
        .expect("a live terminal");
    let b = Timeline::record(back.events);
    narrow
        .terminal_resize(terminal.clone(), 90, 20)
        .await
        .expect("the watcher reports its fit");
    narrow
        .terminal_focus(terminal.clone())
        .await
        .expect("focus the watching desktop");
    let focused = a.wait_for("SIZE 20 90").await && b.wait_for_size([90, 20]).await;
    j.guard(
        "the focused desktop's fit takes over",
        focused && a.size_at("SIZE 20 90") == Some([90, 20]),
        json!({ "size": [90, 20] }),
    );
    wide.terminal_focus(terminal.clone())
        .await
        .expect("focus the first desktop");
    let returned = a.wait_for_size([120, 40]).await
        && b.wait_for_size([120, 40]).await
        && eventually(WAIT, || a.text().matches("SIZE 40 120").count() == 2).await;
    j.guard(
        "and focusing back returns it",
        returned,
        json!({ "size": [120, 40] }),
    );

    let _ = wide.terminal_close(terminal).await;
    j.finish();
}
