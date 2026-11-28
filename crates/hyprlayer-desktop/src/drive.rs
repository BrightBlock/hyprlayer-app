//! The debug-only drive bridge: newline JSON over a Unix socket so a script can find, click and
//! key tracked elements, read the state dump, capture the window and post a notification.
//! Each line is `{"op": …}`; each answer is one line with `"ok"`. Ops are listed in `answer`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Entity, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, OperatingSystem, PlatformInput, Window,
};
use hyprlayer_ui::track::Tracker;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::oneshot;

use crate::desktop::Desktop;
use crate::io::Io;
use crate::single_instance::socket_dir;

/// Holds the socket's path, for a script that knows only the profile.
pub const POINTER: &str = "native-drive-socket";

type Later = Pin<Box<dyn Future<Output = Value> + Send>>;

enum Answer {
    Now(Value),
    /// Runs on the io thread: a subprocess or a blocking call.
    Later(Later),
}

type Ask = (Value, oneshot::Sender<Answer>);

pub fn socket_path(data: &Path) -> PathBuf {
    socket_dir(data).join("drive.sock")
}

pub fn serve(
    data: &Path,
    window: AnyWindowHandle,
    desktop: Entity<Desktop>,
    io: &Io,
    cx: &mut App,
) {
    let path = socket_path(data);
    // A quit skips destructors, so the last run's socket may still be there.
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path)
        .and_then(|l| l.set_nonblocking(true).map(|()| l))
    {
        Ok(listener) => listener,
        Err(e) => {
            log::error!("the drive bridge cannot listen on {}: {e}", path.display());
            return;
        }
    };
    let _ = std::fs::write(data.join(POINTER), path.display().to_string());
    log::info!("drive bridge listening on {}", path.display());
    let (tx, mut rx) = unbounded_channel::<Ask>();
    io.handle().spawn(async move {
        let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
            return;
        };
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(converse(stream, tx.clone()));
        }
    });
    cx.spawn(async move |cx| {
        while let Some((request, reply)) = rx.recv().await {
            let answer = cx.update(|cx| answer(&request, window, &desktop, cx));
            let _ = reply.send(answer);
        }
    })
    .detach();
}

async fn converse(stream: tokio::net::UnixStream, asks: UnboundedSender<Ask>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let (tx, rx) = oneshot::channel();
                if asks.send((request, tx)).is_err() {
                    return;
                }
                match rx.await {
                    Ok(Answer::Now(value)) => value,
                    Ok(Answer::Later(later)) => later.await,
                    Err(_) => failed("the app is shutting down"),
                }
            }
            Err(e) => failed(&format!("not JSON: {e}")),
        };
        if write
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .is_err()
        {
            return;
        }
    }
}

fn failed(error: &str) -> Value {
    json!({ "ok": false, "error": error })
}

fn text<'a>(request: &'a Value, key: &str) -> Option<&'a str> {
    request.get(key).and_then(Value::as_str)
}

fn answer(
    request: &Value,
    window: AnyWindowHandle,
    desktop: &Entity<Desktop>,
    cx: &mut App,
) -> Answer {
    let op = text(request, "op").unwrap_or_default();
    let id = text(request, "id").unwrap_or_default();
    Answer::Now(match op {
        "ids" => {
            let ids: Vec<Value> = Tracker::ids(cx)
                .into_iter()
                .map(|(id, b)| json!({ "id": id, "bounds": bounds(b) }))
                .collect();
            json!({ "ok": true, "ids": ids })
        }
        "find" => match Tracker::find(cx, id) {
            Some(b) => json!({ "ok": true, "id": id, "bounds": bounds(b) }),
            None => failed(&format!("{id} is not painted")),
        },
        "click" => match Tracker::find(cx, id) {
            Some(b) => {
                let at = b.center();
                let clicked = window
                    .update(cx, |_, window, cx| click(window, at, cx))
                    .is_ok();
                json!({ "ok": clicked, "id": id, "at": [f32::from(at.x), f32::from(at.y)] })
            }
            None => failed(&format!("{id} is not painted")),
        },
        "key" => {
            let keystroke = text(request, "keystroke").unwrap_or_default().to_string();
            if text(request, "mode") == Some("compositor") {
                return Answer::Later(Box::pin(compositor_key(keystroke)));
            }
            let os = if cfg!(target_os = "macos") {
                OperatingSystem::Mac
            } else {
                OperatingSystem::Linux
            };
            match Keystroke::parse_with_os(&keystroke, os) {
                Ok(key) => {
                    let handled = window
                        .update(cx, |_, window, cx| window.dispatch_keystroke(key, cx))
                        .unwrap_or(false);
                    json!({ "ok": true, "keystroke": keystroke, "handled": handled })
                }
                Err(e) => failed(&format!("{keystroke}: {e}")),
            }
        }
        "state" => {
            let mut state = desktop.read(cx).dump(cx);
            state["ok"] = json!(true);
            state
        }
        "capture" => {
            let Some(path) = text(request, "path").map(str::to_string) else {
                return Answer::Now(failed("capture needs a path"));
            };
            return Answer::Later(capture(path, window, cx));
        }
        "notify" => {
            let title = text(request, "title").unwrap_or("Hyprlayer").to_string();
            let body = text(request, "body").unwrap_or_default().to_string();
            return Answer::Later(Box::pin(async move {
                let sent =
                    tokio::task::spawn_blocking(move || crate::notify::send(&title, &body)).await;
                match sent {
                    Ok(Ok(())) => json!({ "ok": true }),
                    Ok(Err(e)) => failed(&e),
                    Err(e) => failed(&e.to_string()),
                }
            }));
        }
        // Answers first: quitting now would close the socket before the reply is written.
        "quit" => {
            cx.spawn(async |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
            json!({ "ok": true })
        }
        other => failed(&format!(
            "unknown op {other:?}; ops: ids, find, click, key, state, capture, notify, quit"
        )),
    })
}

fn bounds(b: gpui::Bounds<gpui::Pixels>) -> Value {
    json!({
        "x": f32::from(b.origin.x),
        "y": f32::from(b.origin.y),
        "w": f32::from(b.size.width),
        "h": f32::from(b.size.height),
    })
}

fn click(window: &mut Window, position: gpui::Point<gpui::Pixels>, cx: &mut App) {
    let modifiers = Modifiers::default();
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
        }),
        cx,
    );
}

async fn run(program: &str, args: &[String]) -> Value {
    match tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
    {
        Ok(out) => json!({
            "ok": out.status.success(),
            "status": out.status.code(),
            "stdout": String::from_utf8_lossy(&out.stdout).trim(),
            "stderr": String::from_utf8_lossy(&out.stderr).trim(),
        }),
        Err(e) => failed(&format!("{program}: {e}")),
    }
}

#[cfg(target_os = "macos")]
fn capture(path: String, window: AnyWindowHandle, cx: &mut App) -> Later {
    let number = window
        .update(cx, |_, window, _| window_number(window))
        .ok()
        .flatten();
    Box::pin(async move {
        let Some(number) = number else {
            return failed("no AppKit window behind this GPUI window");
        };
        let mut done = run(
            "screencapture",
            &[
                "-x".into(),
                "-o".into(),
                format!("-l{number}"),
                path.clone(),
            ],
        )
        .await;
        done["path"] = json!(path);
        done
    })
}

/// The CGWindowID `screencapture -l` takes.
#[cfg(target_os = "macos")]
fn window_number(window: &Window) -> Option<isize> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // `Window` has an inherent `window_handle` too; this is raw-window-handle's.
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: GPUI's live content view, read on the main thread that owns it.
    let view = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
    Some(view.window()?.windowNumber())
}

/// Wayland under Hyprland only: the compositor knows where the window is.
#[cfg(not(target_os = "macos"))]
fn capture(path: String, _window: AnyWindowHandle, _cx: &mut App) -> Later {
    Box::pin(async move {
        let geometry = match hyprland_geometry().await {
            Ok(geometry) => geometry,
            Err(e) => return failed(&e),
        };
        let mut done = run("grim", &["-g".into(), geometry, path.clone()]).await;
        done["path"] = json!(path);
        done
    })
}

#[cfg(not(target_os = "macos"))]
async fn hyprland_geometry() -> Result<String, String> {
    let clients = run("hyprctl", &["clients".into(), "-j".into()]).await;
    if clients["ok"] != json!(true) {
        return Err(format!("hyprctl clients failed: {clients}"));
    }
    let list: Value = serde_json::from_str(clients["stdout"].as_str().unwrap_or_default())
        .map_err(|e| format!("hyprctl clients -j: {e}"))?;
    let pid = std::process::id();
    let client = list
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["pid"].as_u64() == Some(u64::from(pid)))
        .ok_or_else(|| format!("Hyprland lists no window for pid {pid}"))?;
    let n = |v: &Value, i: usize| v[i].as_i64().unwrap_or_default();
    let (at, size) = (&client["at"], &client["size"]);
    Ok(format!(
        "{},{} {}x{}",
        n(at, 0),
        n(at, 1),
        n(size, 0),
        n(size, 1)
    ))
}

/// A key the compositor delivers, with a real input serial, which Wayland requires before it
/// lets a client set the clipboard. Hyprland only; the G1 record proved the Lua form.
async fn compositor_key(keystroke: String) -> Value {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return failed("compositor keys need Hyprland (HYPRLAND_INSTANCE_SIGNATURE is unset)");
    }
    let mut parts: Vec<&str> = keystroke.split('-').collect();
    let key = parts.pop().unwrap_or_default();
    let mods: Vec<&str> = parts
        .into_iter()
        .map(|m| match m {
            "ctrl" => "CTRL",
            "shift" => "SHIFT",
            "alt" => "ALT",
            "cmd" | "super" | "platform" => "SUPER",
            other => other,
        })
        .collect();
    let key = match key {
        "enter" => "Return".to_string(),
        "escape" => "Escape".to_string(),
        "tab" => "Tab".to_string(),
        "space" => "space".to_string(),
        "backspace" => "BackSpace".to_string(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_string(),
    };
    let pid = std::process::id();
    let lua = format!(
        "hl.dsp.send_shortcut({{ mods = \"{}\", key = \"{key}\", window = \"pid:{pid}\" }})",
        mods.join(" ")
    );
    let mut done = run("hyprctl", &["dispatch".into(), lua.clone()]).await;
    done["dispatched"] = json!(lua);
    done
}
