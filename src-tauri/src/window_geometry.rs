use tauri::{AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition};

use hyprlayer_client::geometry::{
    decide, parse_saved, MonitorDesc, SavedWindow, DEFAULT_H, DEFAULT_W, MIN_H, MIN_W,
};

fn monitor_desc(m: &Monitor) -> MonitorDesc {
    let pos = m.position();
    let size = m.size();
    let wa = m.work_area();
    let (work_x, work_y, work_w, work_h) = if wa.size.width == 0 || wa.size.height == 0 {
        (pos.x, pos.y, size.width, size.height)
    } else {
        (wa.position.x, wa.position.y, wa.size.width, wa.size.height)
    };
    MonitorDesc {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
        work_x,
        work_y,
        work_w,
        work_h,
        scale: m.scale_factor(),
    }
}

pub fn restore_main_window(app: &AppHandle) {
    if let Err(e) = try_restore(app) {
        log::warn!("main window geometry restore skipped: {e}");
    }
}

fn try_restore(app: &AppHandle) -> tauri::Result<()> {
    let Some(win) = app.get_webview_window("main") else {
        return Ok(());
    };

    let monitors: Vec<MonitorDesc> = win.available_monitors()?.iter().map(monitor_desc).collect();
    if monitors.is_empty() {
        return Ok(());
    }
    let current = win
        .current_monitor()?
        .or(win.primary_monitor()?)
        .map(|m| monitor_desc(&m))
        .unwrap_or(monitors[0]);

    let (min_w, min_h, def_w, def_h) = config_dims(app);
    let saved = load_saved(app);
    let plan = decide(saved, &monitors, current, min_w, min_h, def_w, def_h);

    log::debug!(
        "window geometry restore: saved={saved:?} current={current:?} monitors={monitors:?} plan={plan:?}"
    );

    let (x, y, w, h) = plan.rect();
    // Position is physical; logical sizing avoids stale monitor DPI.
    win.set_position(PhysicalPosition { x, y })?;
    win.set_size(LogicalSize {
        width: w,
        height: h,
    })?;

    if saved.is_some_and(|s| s.maximized) {
        win.maximize()?;
    }
    Ok(())
}

fn config_dims(app: &AppHandle) -> (f64, f64, f64, f64) {
    match app.config().app.windows.iter().find(|w| w.label == "main") {
        Some(w) => (
            w.min_width.unwrap_or(MIN_W),
            w.min_height.unwrap_or(MIN_H),
            w.width,
            w.height,
        ),
        None => (MIN_W, MIN_H, DEFAULT_W, DEFAULT_H),
    }
}

fn load_saved(app: &AppHandle) -> Option<SavedWindow> {
    let path = app
        .path()
        .app_config_dir()
        .ok()?
        .join(tauri_plugin_window_state::DEFAULT_FILENAME);
    parse_saved(&std::fs::read_to_string(path).ok()?)
}
