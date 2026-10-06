//! macOS traffic lights that follow app zoom. Tauri 2 only sets their inset at build time,
//! and AppKit resets them on relayout, so the current inset is re-applied on window events.

#[cfg(target_os = "macos")]
mod imp {
    use std::ptr::NonNull;
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSWindow, NSWindowButton, NSWindowDidBecomeKeyNotification,
        NSWindowDidExitFullScreenNotification, NSWindowDidResizeNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter};

    // Matches the frontend's inset at 100% zoom.
    static INSET: Mutex<(f64, f64)> = Mutex::new((18.0, 20.0));

    fn current() -> (f64, f64) {
        *INSET.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Same geometry as tao's build-time `inset_traffic_lights`.
    fn position(window: &NSWindow, (x, y): (f64, f64)) {
        let (Some(close), Some(mini), Some(zoom)) = (
            window.standardWindowButton(NSWindowButton::CloseButton),
            window.standardWindowButton(NSWindowButton::MiniaturizeButton),
            window.standardWindowButton(NSWindowButton::ZoomButton),
        ) else {
            return;
        };
        // SAFETY: the buttons sit in AppKit's titlebar view hierarchy for the window's life.
        let Some(container) = (unsafe { close.superview().and_then(|v| v.superview()) }) else {
            return;
        };
        let close_rect = close.frame();
        let bar_height = close_rect.size.height + y;
        let mut bar = container.frame();
        bar.size.height = bar_height;
        bar.origin.y = window.frame().size.height - bar_height;
        container.setFrame(bar);

        let spacing = mini.frame().origin.x - close_rect.origin.x;
        for (i, button) in [close, mini, zoom].iter().enumerate() {
            let mut origin = button.frame().origin;
            origin.x = x + i as f64 * spacing;
            button.setFrameOrigin(origin);
        }
    }

    fn ns_window(window: &tauri::WebviewWindow) -> Option<Retained<NSWindow>> {
        let ptr = window.ns_window().ok()? as *mut NSWindow;
        // SAFETY: tauri hands back the live NSWindow backing this webview window.
        unsafe { Retained::retain(ptr) }
    }

    pub fn install(window: &tauri::WebviewWindow) {
        let Some(ns_window) = ns_window(window) else {
            log::warn!(
                "no NSWindow for {}; traffic lights stay put",
                window.label()
            );
            return;
        };
        position(&ns_window, current());

        let block = RcBlock::new(|note: NonNull<NSNotification>| {
            // SAFETY: AppKit passes a valid notification for the block's lifetime.
            let note = unsafe { note.as_ref() };
            if let Some(window) = note.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                position(&window, current());
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        for name in [
            // SAFETY: AppKit's notification-name constants are immutable statics.
            unsafe { NSWindowDidResizeNotification },
            unsafe { NSWindowDidExitFullScreenNotification },
            unsafe { NSWindowDidBecomeKeyNotification },
        ] {
            // SAFETY: main-thread observer scoped to this window; the window outlives the app.
            let token = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    Some(&ns_window),
                    None,
                    &block,
                )
            };
            std::mem::forget(token);
        }
    }

    pub fn set_inset(window: &tauri::WebviewWindow, x: f64, y: f64) {
        *INSET.lock().unwrap_or_else(|e| e.into_inner()) = (x, y);
        if let Some(ns_window) = ns_window(window) {
            position(&ns_window, (x, y));
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{install, set_inset};

#[cfg(not(target_os = "macos"))]
pub fn install(_window: &tauri::WebviewWindow) {}

#[cfg(not(target_os = "macos"))]
pub fn set_inset(_window: &tauri::WebviewWindow, _x: f64, _y: f64) {}
