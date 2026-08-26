use std::collections::HashMap;

use tauri::{AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition};

const DEFAULT_W: f64 = 1280.0;
const DEFAULT_H: f64 = 800.0;
const MIN_W: f64 = 1180.0;
const MIN_H: f64 = 620.0;

const DEFAULT_FRACTION: f64 = 0.75;
const MAX_DEFAULT_W: f64 = 2560.0;
const MAX_DEFAULT_H: f64 = 1600.0;
const MAX_WORK_FRACTION: f64 = 0.94;

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(default)]
struct SavedWindow {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    prev_x: i32,
    prev_y: i32,
    maximized: bool,
}

#[derive(Debug, Clone, Copy)]
struct MonitorDesc {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    work_x: i32,
    work_y: i32,
    work_w: u32,
    work_h: u32,
    scale: f64,
}

impl MonitorDesc {
    fn from_monitor(m: &Monitor) -> Self {
        let pos = m.position();
        let size = m.size();
        let wa = m.work_area();
        let (work_x, work_y, work_w, work_h) = if wa.size.width == 0 || wa.size.height == 0 {
            (pos.x, pos.y, size.width, size.height)
        } else {
            (wa.position.x, wa.position.y, wa.size.width, wa.size.height)
        };
        Self {
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Plan {
    RestoreExact { x: i32, y: i32, w: f64, h: f64 },
    DefaultCentered { x: i32, y: i32, w: f64, h: f64 },
}

impl Plan {
    fn rect(self) -> (i32, i32, f64, f64) {
        match self {
            Plan::RestoreExact { x, y, w, h } | Plan::DefaultCentered { x, y, w, h } => {
                (x, y, w, h)
            }
        }
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

    let monitors: Vec<MonitorDesc> = win
        .available_monitors()?
        .iter()
        .map(MonitorDesc::from_monitor)
        .collect();
    if monitors.is_empty() {
        return Ok(());
    }
    let current = win
        .current_monitor()?
        .or(win.primary_monitor()?)
        .map(|m| MonitorDesc::from_monitor(&m))
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

    if saved.map(|s| s.maximized).unwrap_or(false) {
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
    let raw = std::fs::read_to_string(path).ok()?;
    let mut map: HashMap<String, SavedWindow> = serde_json::from_str(&raw).ok()?;
    let s = map.remove("main")?;
    (s.width != 0 && s.height != 0).then_some(s)
}

fn decide(
    saved: Option<SavedWindow>,
    monitors: &[MonitorDesc],
    current: MonitorDesc,
    min_w: f64,
    min_h: f64,
    def_w: f64,
    def_h: f64,
) -> Plan {
    if let Some(s) = saved {
        let (px, py) = if s.maximized {
            (s.prev_x, s.prev_y)
        } else {
            (s.x, s.y)
        };
        if let Some(m) = monitors
            .iter()
            .find(|m| intersects(m, px, py, s.width, s.height))
        {
            let lw = s.width as f64 / m.scale;
            let lh = s.height as f64 / m.scale;
            if lw >= min_w - 1.0 && lh >= min_h - 1.0 {
                let (x, y, w, h) = clamp_to_work_area(px, py, s.width, s.height, m);
                return Plan::RestoreExact {
                    x,
                    y,
                    w: w as f64 / m.scale,
                    h: h as f64 / m.scale,
                };
            }
            return default_on(m, min_w, min_h, def_w, def_h);
        }
    }
    default_on(&current, min_w, min_h, def_w, def_h)
}

fn intersects(m: &MonitorDesc, x: i32, y: i32, w: u32, h: u32) -> bool {
    let (left, right, top, bottom) = (m.x, m.x + m.w as i32, m.y, m.y + m.h as i32);
    [
        (x, y),
        (x + w as i32, y),
        (x, y + h as i32),
        (x + w as i32, y + h as i32),
    ]
    .into_iter()
    .any(|(cx, cy)| cx >= left && cx < right && cy >= top && cy < bottom)
}

fn clamp_to_work_area(x: i32, y: i32, w: u32, h: u32, m: &MonitorDesc) -> (i32, i32, u32, u32) {
    let w = w.min(m.work_w);
    let h = h.min(m.work_h);
    let max_x = m.work_x + (m.work_w as i32 - w as i32);
    let max_y = m.work_y + (m.work_h as i32 - h as i32);
    let x = x.clamp(m.work_x, max_x.max(m.work_x));
    let y = y.clamp(m.work_y, max_y.max(m.work_y));
    (x, y, w, h)
}

fn default_on(m: &MonitorDesc, min_w: f64, min_h: f64, def_w: f64, def_h: f64) -> Plan {
    let w_l = default_axis(m.work_w as f64 / m.scale, min_w, def_w, MAX_DEFAULT_W);
    let h_l = default_axis(m.work_h as f64 / m.scale, min_h, def_h, MAX_DEFAULT_H);
    let w_px = (w_l * m.scale).round() as i32;
    let h_px = (h_l * m.scale).round() as i32;
    let x = m.work_x + (m.work_w as i32 - w_px) / 2;
    let y = m.work_y + (m.work_h as i32 - h_px) / 2;
    Plan::DefaultCentered {
        x,
        y,
        w: w_l,
        h: h_l,
    }
}

fn default_axis(work: f64, min: f64, def: f64, cap: f64) -> f64 {
    (DEFAULT_FRACTION * work)
        .max(def)
        .min(cap)
        .min(MAX_WORK_FRACTION * work)
        .max(min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorDesc {
        MonitorDesc {
            x,
            y,
            w,
            h,
            work_x: x,
            work_y: y,
            work_w: w,
            work_h: h,
            scale,
        }
    }

    fn saved(x: i32, y: i32, w: u32, h: u32) -> SavedWindow {
        SavedWindow {
            width: w,
            height: h,
            x,
            y,
            prev_x: x,
            prev_y: y,
            maximized: false,
        }
    }

    const DIMS: (f64, f64, f64, f64) = (MIN_W, MIN_H, DEFAULT_W, DEFAULT_H);

    fn decide_std(s: Option<SavedWindow>, mons: &[MonitorDesc], cur: MonitorDesc) -> Plan {
        decide(s, mons, cur, DIMS.0, DIMS.1, DIMS.2, DIMS.3)
    }

    #[test]
    fn restores_verbatim_when_saved_monitor_present_same_dpi() {
        let m = mon(0, 0, 1920, 1080, 1.0);
        let s = saved(200, 150, 1400, 900);
        assert_eq!(
            decide_std(Some(s), &[m], m),
            Plan::RestoreExact {
                x: 200,
                y: 150,
                w: 1400.0,
                h: 900.0
            }
        );
    }

    #[test]
    fn restore_size_is_logical_on_a_2x_monitor() {
        let m = mon(0, 0, 3024, 1964, 2.0);
        let s = saved(100, 100, 2400, 1600);
        assert_eq!(
            decide_std(Some(s), &[m], m),
            Plan::RestoreExact {
                x: 100,
                y: 100,
                w: 1200.0,
                h: 800.0
            }
        );
    }

    #[test]
    fn off_screen_saved_falls_back_to_centered_default_on_current() {
        let built_in = mon(0, 0, 3024, 1964, 2.0);
        let s = saved(-3249, -888, 1934, 1109);
        assert_eq!(
            decide_std(Some(s), &[built_in], built_in),
            Plan::DefaultCentered {
                x: 232,
                y: 182,
                w: 1280.0,
                h: 800.0
            }
        );
    }

    #[test]
    fn sub_min_at_target_dpi_falls_back_to_default() {
        let m = mon(0, 0, 1920, 1080, 2.0);
        let s = saved(100, 100, 1200, 700);
        assert!(matches!(
            decide_std(Some(s), &[m], m),
            Plan::DefaultCentered { .. }
        ));
    }

    #[test]
    fn oversized_saved_is_clamped_to_work_area() {
        let m = mon(0, 0, 1920, 1080, 1.0);
        let s = saved(0, 0, 4000, 3000);
        assert_eq!(
            decide_std(Some(s), &[m], m),
            Plan::RestoreExact {
                x: 0,
                y: 0,
                w: 1920.0,
                h: 1080.0
            }
        );
    }

    #[test]
    fn no_saved_state_scales_default_to_monitor() {
        let m = mon(0, 0, 1920, 1080, 1.0);
        assert_eq!(
            decide_std(None, &[m], m),
            Plan::DefaultCentered {
                x: 240,
                y: 135,
                w: 1440.0,
                h: 810.0
            }
        );
    }

    #[test]
    fn default_scales_up_on_large_non_retina_monitor() {
        let m = mon(0, 0, 3440, 1440, 1.0);
        assert_eq!(
            decide_std(None, &[m], m),
            Plan::DefaultCentered {
                x: 440,
                y: 180,
                w: 2560.0,
                h: 1080.0
            }
        );
    }

    #[test]
    fn picks_the_intersecting_monitor_among_several() {
        let left = mon(-3440, 0, 3440, 1440, 1.0);
        let primary = mon(0, 0, 3024, 1964, 2.0);
        let s = saved(-3000, 200, 1934, 1109);
        assert_eq!(
            decide_std(Some(s), &[primary, left], primary),
            Plan::RestoreExact {
                x: -3000,
                y: 200,
                w: 1934.0,
                h: 1109.0
            }
        );
    }

    #[test]
    fn default_stays_off_the_screen_edges_on_small_monitors() {
        let m = mon(0, 0, 1300, 900, 1.0);
        assert_eq!(
            decide_std(None, &[m], m),
            Plan::DefaultCentered {
                x: 39,
                y: 50,
                w: 1222.0,
                h: 800.0
            }
        );
    }

    #[test]
    fn default_caps_at_max_on_huge_monitors() {
        let m = mon(0, 0, 5120, 2160, 1.0);
        let (_, _, w, h) = decide_std(None, &[m], m).rect();
        assert_eq!((w, h), (MAX_DEFAULT_W, MAX_DEFAULT_H));
    }

    #[test]
    fn low_min_config_restores_a_tiled_sub_1180_window() {
        let m = mon(0, 0, 2880, 1800, 2.0);
        let s = saved(0, 0, 1402, 1704);
        assert_eq!(
            decide(Some(s), &[m], m, 500.0, 400.0, DEFAULT_W, DEFAULT_H),
            Plan::RestoreExact {
                x: 0,
                y: 0,
                w: 701.0,
                h: 852.0
            }
        );
    }

    #[test]
    fn maximized_uses_prev_coords_for_placement() {
        let external = mon(-3440, 0, 3440, 1440, 1.0);
        let primary = mon(0, 0, 3024, 1964, 2.0);
        let s = SavedWindow {
            width: 1600,
            height: 1000,
            x: -3440,
            y: 0,
            prev_x: -2000,
            prev_y: 100,
            maximized: true,
        };
        assert_eq!(
            decide_std(Some(s), &[primary, external], primary),
            Plan::RestoreExact {
                x: -2000,
                y: 100,
                w: 1600.0,
                h: 1000.0
            }
        );
    }
}
