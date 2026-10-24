const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// WebKitGTK before 2.54.2 turns on Wayland explicit sync for the NVIDIA EGL
/// surface but commits frames without an acquire point, so Hyprland drops the
/// connection (`Error 71 (Protocol error)`) seconds after the first paint
/// (https://bugs.webkit.org/show_bug.cgi?id=280210). Turning the DMA-BUF
/// renderer off avoids that path; `__NV_DISABLE_EXPLICIT_SYNC=1` does too but
/// flickers. WebKit reads the variable once, at GTK init, so this must run
/// before the builder; a value the user set wins.
#[cfg(target_os = "linux")]
pub fn apply() {
    if std::env::var_os(DISABLE_DMABUF).is_none()
        && nvidia_driver_loaded()
        && needs_dmabuf_workaround(webkit_version())
    {
        std::env::set_var(DISABLE_DMABUF, "1");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply() {}

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
fn needs_dmabuf_workaround(version: (u32, u32, u32)) -> bool {
    version < (2, 54, 2)
}

#[cfg(test)]
mod tests {
    use super::needs_dmabuf_workaround;

    #[test]
    fn only_webkit_before_2_54_2_needs_the_workaround() {
        assert!(needs_dmabuf_workaround((2, 52, 5)));
        assert!(needs_dmabuf_workaround((2, 54, 1)));
        assert!(!needs_dmabuf_workaround((2, 54, 2)));
        assert!(!needs_dmabuf_workaround((2, 56, 0)));
    }
}
