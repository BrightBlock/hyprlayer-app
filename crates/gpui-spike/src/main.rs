//! G1 build-gate spike: one window with the five pieces the GPUI plan needs to prove links in one binary.
mod feed;
mod grid;
mod notify;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, point, px, size, App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    PlatformInput, Render, SharedString, Size, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_component::input::{Editor, EditorState};
use gpui_component::text::TextView;
use gpui_component::{v_virtual_list, ActiveTheme as _, Root, StyledExt as _};

use feed::{Msg, Source};
use grid::TermGrid;

const MARKDOWN: &str = "# Transcript item\n\nSelect **this paragraph** and copy it: the spike checks `TextView` selection.\n\n```rust\nfn main() {\n    let answer = 6 * 7; // highlighted by tree-sitter\n    println!(\"{answer}\");\n}\n```\n\n- list item one\n- list item two\n";
const EDITOR_TEXT: &str = "// type here; IME is checked on Wayland\nfn greet(name: &str) -> String {\n    format!(\"hello, {name}\")\n}\n";
const ROWS: usize = 1500;

struct Spike {
    grid: Entity<TermGrid>,
    editor: Entity<EditorState>,
    sizes: Rc<Vec<Size<Pixels>>>,
    ticks: u64,
    gpu: SharedString,
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = {
            let g = self.grid.read(cx);
            format!(
                "gpu: {} | frames {} bytes {} | tokio ticks {} | {}",
                self.gpu, g.frames, g.bytes, self.ticks, g.status
            )
        };
        let rows = v_virtual_list(cx.entity(), "rows", self.sizes.clone(), |_, range, _, cx| {
            range
                .map(|i| {
                    div()
                        .h(px(24.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(format!("row {i:04}: transcript item"))
                })
                .collect()
        });
        div()
            .size_full()
            .bg(cx.theme().background)
            .v_flex()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .w_1_2()
                            .v_flex()
                            .child(div().h_1_2().min_h_0().child(self.grid.clone()))
                            .child(div().h_1_2().min_h_0().child(rows)),
                    )
                    .child(
                        div()
                            .w_1_2()
                            .v_flex()
                            .child(
                                div()
                                    .id("md")
                                    .h_1_2()
                                    .min_h_0()
                                    .p_3()
                                    .child(TextView::markdown("md-view", MARKDOWN).selectable(true).scrollable(true)),
                            )
                            .child(div().h_1_2().min_h_0().child(Editor::new(&self.editor).h_full())),
                    ),
            )
            .child(div().h(px(22.0)).px_2().text_xs().child(status))
    }
}

fn gpu_name(window: &Window) -> String {
    match window.gpu_specs() {
        Some(s) => format!(
            "{} ({}, {}){}",
            s.device_name,
            s.driver_name,
            s.driver_info,
            if s.is_software_emulated { " SOFTWARE" } else { "" }
        ),
        None => format!("{} (no gpu_specs on this backend)", std::env::consts::OS),
    }
}

struct Args {
    source: Source,
    selftest: Option<PathBuf>,
}

fn args() -> Args {
    let mut it = std::env::args().skip(1);
    let (mut socket, mut ws, mut host, mut selftest) = (None, None, false, None);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().map(PathBuf::from),
            "--workstream" => ws = it.next(),
            "--host" => host = true,
            "--selftest" => selftest = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    let source = match (socket, ws, host) {
        (Some(socket), Some(workstream), _) => Source::Attach { socket, workstream },
        (_, _, true) => Source::Host,
        _ => Source::Synthetic,
    };
    Args { source, selftest }
}

fn main() {
    let args = args();
    let rx = feed::spawn(args.source);
    let selftest = args.selftest;
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        let bounds = Bounds::centered(None, size(px(1400.0), px(900.0)), cx);
        let window = cx
            .open_window(
                WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
                |window, cx| {
                    let grid = cx.new(|_| TermGrid::new(100, 24));
                    let editor = cx.new(|cx| EditorState::new(window, cx).language("rust").default_value(EDITOR_TEXT));
                    let gpu: SharedString = gpu_name(window).into();
                    eprintln!("ADAPTER: {gpu}");
                    let sizes = Rc::new(vec![size(px(100.0), px(24.0)); ROWS]);
                    let view = cx.new(|_| Spike { grid, editor, sizes, ticks: 0, gpu });
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("open window");
        let root = window.update(cx, |root, _, _| root.view().clone()).ok();
        let spike: Option<Entity<Spike>> = root.and_then(|v| v.downcast().ok());
        let Some(spike) = spike else { return };

        let feed_spike = spike.clone();
        let mut rx = rx;
        cx.spawn(async move |cx| {
            while let Some(msg) = rx.recv().await {
                let _ = cx.update(|cx| {
                    feed_spike.update(cx, |s, cx| {
                        match msg {
                            Msg::Bytes(b) => s.grid.update(cx, |g, _| g.write(&b)),
                            Msg::Resize(c, r) => s.grid.update(cx, |g, _| g.resize(c, r)),
                            Msg::Status(t) => s.grid.update(cx, |g, _| g.status = t),
                            Msg::Tick(n) => s.ticks = n,
                        }
                        cx.notify();
                    });
                });
            }
        })
        .detach();

        if let Some(out) = selftest {
            selftest_run(cx, window.into(), spike, out);
        }
        cx.activate(true);
    });
}

fn selftest_run(cx: &mut App, window: gpui::AnyWindowHandle, spike: Entity<Spike>, out: PathBuf) {
    cx.spawn(async move |cx| {
        let mut report = serde_json::Map::new();
        eprintln!("selftest: started");
        // A hosted server cold-starts slowly; wait for the first frame, up to 90 s.
        for _ in 0..180 {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            if cx.update(|cx| spike.read(cx).grid.read(cx).frames) > 0 {
                break;
            }
        }
        cx.background_executor().timer(Duration::from_secs(3)).await;
        eprintln!("selftest: timer fired");
        let _ = std::fs::create_dir_all(&out);
        // Drag across the paragraph, a frame between events, then Copy; the clipboard must hold the text.
        let ev = |e: PlatformInput| cx.update(|cx| window.update(cx, |_, w, cx| { w.dispatch_event(e, cx); }).is_ok());
        let down = |p| PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: p, modifiers: Default::default(), click_count: 1, first_mouse: false });
        let mv = |p| PlatformInput::MouseMove(MouseMoveEvent { position: p, pressed_button: Some(MouseButton::Left), modifiers: Default::default() });
        let up = |p| PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: p, modifiers: Default::default(), click_count: 1 });
        let width = cx.update(|cx| window.update(cx, |_, w, _| w.bounds().size.width).ok()).unwrap_or(px(1400.0));
        // The markdown pane is the right half; its paragraph sits ~70 px down.
        let left = width / 2.0 + px(12.0);
        let (a, b) = (point(left, px(70.0)), point(left + px(290.0), px(72.0)));
        let step = Duration::from_millis(60);
        let mut drag = ev(mv(a));
        cx.background_executor().timer(step).await;
        drag &= ev(down(a));
        for i in 1..=10 {
            cx.background_executor().timer(step).await;
            let t = i as f32 / 10.0;
            drag &= ev(mv(point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)));
        }
        drag &= ev(up(b));
        cx.background_executor().timer(step).await;
        let key = if cfg!(target_os = "macos") { "cmd-c" } else { "ctrl-c" };
        cx.update(|cx| window.update(cx, |_, w, cx| { w.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx); }).ok());
        eprintln!("selftest: dragged {drag}");
        cx.background_executor().timer(Duration::from_millis(500)).await;
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
        report.insert("selection_copy".into(), serde_json::json!({ "drag_ok": drag, "clipboard": copied }));
        // Terminal grid text proves the live (or synthetic) frames reached ghostty-vt.
        let text = cx.update(|cx| spike.read(cx).grid.read(cx).text());
        eprintln!("selftest: clipboard read");
        report.insert("grid_text".into(), serde_json::json!(text));
        report.insert("status".into(), serde_json::json!(cx.update(|cx| spike.read(cx).grid.read(cx).status.clone())));
        report.insert("ticks".into(), serde_json::json!(cx.update(|cx| spike.read(cx).ticks)));
        // Notification.
        eprintln!("selftest: notifying");
        let (ntx, nrx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = ntx.send(notify::send("Hyprlayer spike", "gpui build gate"));
        });
        let mut note = "no result within 20s".to_string();
        for _ in 0..40 {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            match nrx.try_recv() {
                Ok(Ok(())) => { note = "sent".into(); break; }
                Ok(Err(e)) => { note = format!("error: {e}"); break; }
                Err(_) => {}
            }
        }
        report.insert("notification".into(), serde_json::json!(note));
        let _ = std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&report).unwrap());
        if let Some(hold) = std::env::var("HYPRLAYER_SPIKE_HOLD").ok().and_then(|v| v.parse::<u64>().ok()) {
            cx.background_executor().timer(Duration::from_secs(hold)).await;
        }
        let pid = feed::HOSTED_PID.load(std::sync::atomic::Ordering::Relaxed);
        if pid != 0 {
            let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
