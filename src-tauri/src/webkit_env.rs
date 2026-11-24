#[cfg(target_os = "linux")]
const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

#[cfg(target_os = "linux")]
static SET_BY_APP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// WebKitGTK before 2.54.2 turns on Wayland explicit sync for the NVIDIA EGL
/// surface but commits frames without an acquire point, so Hyprland drops the
/// connection (`Error 71 (Protocol error)`) seconds after the first paint
/// (https://bugs.webkit.org/show_bug.cgi?id=280210). Turning the DMA-BUF
/// renderer off avoids that path; `__NV_DISABLE_EXPLICIT_SYNC=1` does too but
/// flickers. WebKit reads the variable once, at GTK init, so this must run
/// before the builder; a value the user set wins.
#[cfg(target_os = "linux")]
pub fn apply() {
    let user_set = std::env::var_os(DISABLE_DMABUF).is_some();
    if sets_dmabuf_workaround(user_set, nvidia_driver_loaded, webkit_version) {
        std::env::set_var(DISABLE_DMABUF, "1");
        SET_BY_APP.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply() {}

/// The variables `apply` set for this process's own webview. They are meant for
/// it alone: the server and every agent it starts must not inherit them, or a
/// debug app an agent launches runs WebKit in fallback mode. A value the user
/// set before launch is not listed and passes through.
#[cfg(target_os = "linux")]
pub fn set_by_app() -> Vec<&'static str> {
    if SET_BY_APP.load(std::sync::atomic::Ordering::Relaxed) {
        vec![DISABLE_DMABUF]
    } else {
        Vec::new()
    }
}

#[cfg(not(target_os = "linux"))]
pub fn set_by_app() -> Vec<&'static str> {
    Vec::new()
}

/// Without the DMA-BUF renderer (or with compositing off) WebKitGTK composites
/// nothing: every frame of an endless CSS animation repaints and copies the
/// whole window on the web process's main thread, which held it at 35–85%
/// while an agent worked. The page shows those indicators still when this is true.
#[cfg(target_os = "linux")]
pub fn still_motion() -> bool {
    [DISABLE_DMABUF, "WEBKIT_DISABLE_COMPOSITING_MODE"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| v != "0"))
}

#[cfg(not(target_os = "linux"))]
pub fn still_motion() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn nvidia_driver_loaded() -> bool {
    std::path::Path::new("/sys/module/nvidia").exists()
}

#[cfg(target_os = "linux")]
fn webkit_version() -> (u32, u32, u32) {
    // SAFETY: plain getters for the loaded library's version; they need no
    // GTK or WebKit initialisation.
    unsafe {
        (
            webkit2gtk_sys::webkit_get_major_version(),
            webkit2gtk_sys::webkit_get_minor_version(),
            webkit2gtk_sys::webkit_get_micro_version(),
        )
    }
}

#[cfg(any(target_os = "linux", test))]
fn sets_dmabuf_workaround(
    user_set: bool,
    nvidia: impl FnOnce() -> bool,
    version: impl FnOnce() -> (u32, u32, u32),
) -> bool {
    !user_set && nvidia() && needs_dmabuf_workaround(version())
}

#[cfg(any(target_os = "linux", test))]
fn needs_dmabuf_workaround(version: (u32, u32, u32)) -> bool {
    version < (2, 54, 2)
}

#[cfg(test)]
mod tests {
    use super::{needs_dmabuf_workaround, sets_dmabuf_workaround};

    #[test]
    fn a_value_the_user_set_is_never_claimed_by_the_app() {
        let old = || (2, 52, 5);
        assert!(sets_dmabuf_workaround(false, || true, old));
        assert!(!sets_dmabuf_workaround(true, || true, old));
        assert!(!sets_dmabuf_workaround(false, || false, old));
    }

    #[test]
    fn only_webkit_before_2_54_2_needs_the_workaround() {
        assert!(needs_dmabuf_workaround((2, 52, 5)));
        assert!(needs_dmabuf_workaround((2, 54, 1)));
        assert!(!needs_dmabuf_workaround((2, 54, 2)));
        assert!(!needs_dmabuf_workaround((2, 56, 0)));
    }
}
