//! Trims the gestures WebKitGTK installs on the web view behind the app's back.
//!
//! No-op everywhere but Linux; the other two webviews don't take these gestures
//! (WKWebView's `allowsMagnification` is off by default, and wry leaves
//! WebView2's zoom control alone unless `zoom_hotkeys_enabled` is set, which
//! this app never sets).

/// Disable WebKitGTK's built-in pinch-to-zoom.
///
/// A two-finger trackpad pinch magnified the whole app the way it would a web
/// page, and the scale stuck after the fingers lifted. WebKitGTK installs a
/// `GtkGestureZoom` on the web view widget and wires `scale-changed` to
/// `ViewGestureController::setMagnification`
/// (`WebKitWebViewBase.cpp`, `webkitWebViewBaseZoomChanged`); the same gesture
/// object covers touchscreen pinch. It is unconditional upstream — there is no
/// `WebKitSettings` property for it, and no DOM event to `preventDefault()`,
/// because the magnification is applied in the UI process before the page sees
/// anything. So the only place to stop it is the GTK widget.
///
/// Setting the controller's propagation phase to `None` leaves the gesture
/// object alive and merely stops GTK feeding it events. The obvious
/// alternative — destroying its signal handlers or clearing the
/// `wk-view-zoom-gesture` object data (the workaround in wry#544) — frees a
/// gesture that `WebKitWebViewBasePrivate::touchGestureGroup` still points at,
/// and that the long-press gesture is grouped with, which segfaults as soon as
/// a page prevents an event.
///
/// Degrades to a log line if the key ever disappears upstream: losing the fix
/// is not worth failing startup over.
#[cfg(target_os = "linux")]
pub fn disable_pinch_zoom(window: &tauri::WebviewWindow) {
    if let Err(e) = window.with_webview(|webview| {
        use gtk::glib::translate::from_glib_none;
        use gtk::prelude::*;

        let view = webview.inner();
        // Set by `webkit_web_view_base_init` with `g_object_set_data_full`, so
        // the quark holds the raw `GtkGestureZoom*`, not a Rust wrapper.
        let gesture: Option<gtk::EventController> = unsafe {
            view.data::<gtk::ffi::GtkEventController>("wk-view-zoom-gesture")
                .map(|p| from_glib_none(p.as_ptr()))
        };
        match gesture {
            Some(gesture) => gesture.set_propagation_phase(gtk::PropagationPhase::None),
            None => log::warn!("no wk-view-zoom-gesture on the webview; pinch zoom stays enabled"),
        }
    }) {
        log::warn!("could not reach the GTK webview to disable pinch zoom: {e}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn disable_pinch_zoom(_window: &tauri::WebviewWindow) {}
