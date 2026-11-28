//! The app around the root view: the window and its saved bounds, the macOS menu, the window
//! class, and quitting.

use std::path::{Path, PathBuf};

use gpui::{
    actions, point, px, size, App, AppContext as _, Bounds, Entity, KeyBinding, Menu, MenuItem,
    OsAction, PlatformDisplay, SystemMenuType, Window, WindowBounds, WindowHandle, WindowOptions,
};
use gpui_component::{Root, TitleBar};
use hyprlayer_client::geometry::{
    self, MonitorDesc, SavedWindow, DEFAULT_H, DEFAULT_W, MIN_H, MIN_W,
};
use hyprlayer_client::supervisor;
use hyprlayer_server::APP_IDENTIFIER;
use hyprlayer_ui::icon::Assets;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::config::Config;
use crate::desktop::Desktop;
use crate::io::Io;
use crate::single_instance::Guard;

actions!(
    hyprlayer,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        ToggleFullScreen,
        RevealLogs
    ]
);

/// Not the window-state plugin's `.window-state.json`: the Tauri app owns that one.
const WINDOW_STATE: &str = "native-window.json";

/// A compositor rule or launcher's `StartupWMClass` tells the release from every other build.
pub fn app_id(identifier: &str) -> &'static str {
    if identifier == APP_IDENTIFIER {
        "Hyprlayer"
    } else {
        "Hyprlayer-Dev"
    }
}

/// What a window needs before it opens; the E2E tier calls it too.
pub fn init(cx: &mut App) {
    gpui_component::init(cx);
}

pub fn run(config: Config, io: Io, instance: Guard) {
    let focus_requests = instance.serve(&io);
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            init(cx);
            bind(config.paths.log.clone(), cx);
            let (window, desktop) = match open(config.clone(), io.clone(), cx) {
                Ok(opened) => opened,
                Err(e) => {
                    log::error!("could not open the window: {e}");
                    cx.quit();
                    return;
                }
            };
            follow_focus_requests(focus_requests, window, cx);
            #[cfg(all(debug_assertions, unix))]
            crate::drive::serve(&config.paths.data, window.into(), desktop, &io, cx);
            #[cfg(not(all(debug_assertions, unix)))]
            drop(desktop);
            cx.activate(true);
        });
}

pub fn open(
    config: Config,
    io: Io,
    cx: &mut App,
) -> gpui::Result<(WindowHandle<Root>, Entity<Desktop>)> {
    let state = config.paths.config.join(WINDOW_STATE);
    let options = WindowOptions {
        window_bounds: Some(restore(&state, cx)),
        window_min_size: Some(size(px(MIN_W as f32), px(MIN_H as f32))),
        app_id: Some(app_id(&config.identifier).to_string()),
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui::WindowDecorations::Client),
        ..TitleBar::window_options()
    };
    let mut desktop = None;
    let saved = state.clone();
    let window = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| Desktop::new(config, io, window, cx));
        desktop = Some(view.clone());
        // Closing the window quits, as the Tauri window's close does.
        window.on_window_should_close(cx, move |window, cx| {
            save(&saved, window);
            cx.quit();
            false
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    // `cx.quit()` runs quit observers but no destructors, so `kill_on_drop` never ends a relay.
    cx.on_app_quit(move |cx| {
        let _ = window.update(cx, |_, window, _| save(&state, window));
        supervisor::stop_relays();
        async {}
    })
    .detach();
    let desktop = desktop.expect("the window builds its root view");
    Ok((window, desktop))
}

fn follow_focus_requests(
    mut requests: UnboundedReceiver<()>,
    window: WindowHandle<Root>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while requests.recv().await.is_some() {
            let _ = cx.update(|cx| {
                cx.activate(true);
                window.update(cx, |_, window, _| window.activate_window())
            });
        }
    })
    .detach();
}

fn monitor(display: &dyn PlatformDisplay) -> MonitorDesc {
    let (b, v) = (display.bounds(), display.visible_bounds());
    let int = |p: gpui::Pixels| f32::from(p).round() as i32;
    let len = |p: gpui::Pixels| f32::from(p).round().max(0.0) as u32;
    MonitorDesc {
        x: int(b.origin.x),
        y: int(b.origin.y),
        w: len(b.size.width),
        h: len(b.size.height),
        work_x: int(v.origin.x),
        work_y: int(v.origin.y),
        work_w: len(v.size.width),
        work_h: len(v.size.height),
        // GPUI reports displays in logical pixels, and the saved bounds are logical too.
        scale: 1.0,
    }
}

/// `geometry::decide` over this client's own saved bounds.
fn restore(path: &Path, cx: &App) -> WindowBounds {
    let saved = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| geometry::parse_saved(&raw));
    let monitors: Vec<MonitorDesc> = cx.displays().iter().map(|d| monitor(&**d)).collect();
    let current = cx
        .primary_display()
        .map(|d| monitor(&*d))
        .or_else(|| monitors.first().copied());
    let Some(current) = current else {
        let fallback = size(px(DEFAULT_W as f32), px(DEFAULT_H as f32));
        return WindowBounds::Windowed(Bounds::centered(None, fallback, cx));
    };
    let plan = geometry::decide(
        saved, &monitors, current, MIN_W, MIN_H, DEFAULT_W, DEFAULT_H,
    );
    let (x, y, w, h) = plan.rect();
    let bounds = Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(w as f32), px(h as f32)),
    );
    if saved.is_some_and(|s| s.maximized) {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

fn save(path: &Path, window: &Window) {
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(b) => (b, false),
        WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
    };
    let int = |p: gpui::Pixels| f32::from(p).round() as i32;
    let (x, y) = (int(bounds.origin.x), int(bounds.origin.y));
    let saved = SavedWindow {
        width: int(bounds.size.width).max(0) as u32,
        height: int(bounds.size.height).max(0) as u32,
        x,
        y,
        prev_x: x,
        prev_y: y,
        maximized,
    };
    let file = json!({ "main": saved });
    if let Err(e) = std::fs::write(path, file.to_string()) {
        log::warn!(
            "could not save the window bounds to {}: {e}",
            path.display()
        );
    }
}

fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window)) {
    if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_, window, _| f(window));
    }
}

/// The Tauri menu's items that have a feature behind them yet; Settings, Save, Check for
/// Updates and Copy Diagnostics arrive with their features.
fn bind(log_dir: PathBuf, cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, |w| w.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_active_window(cx, |w| w.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| with_active_window(cx, |w| w.toggle_fullscreen()));
    cx.on_action(move |_: &RevealLogs, cx| cx.reveal_path(&log_dir));
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        ]);
    } else {
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
    }
    use gpui_component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    cx.set_menus([
        Menu::new("Hyprlayer").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Hyprlayer", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Hyprlayer", Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([MenuItem::action("Enter Full Screen", ToggleFullScreen)]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
        Menu::new("Help").items([MenuItem::action("Reveal Logs", RevealLogs)]),
    ]);
}
