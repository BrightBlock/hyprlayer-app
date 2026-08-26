#[cfg(target_os = "linux")]
pub fn disable_pinch_zoom(window: &tauri::WebviewWindow) {
    if let Err(e) = window.with_webview(|webview| {
        use gtk::glib::translate::from_glib_none;
        use gtk::prelude::*;

        let view = webview.inner();
        // SAFETY: WebKit stores a GtkEventController pointer under this key.
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
