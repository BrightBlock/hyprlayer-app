//! DPI-aware restore of the main window's saved geometry.
//!
//! `tauri-plugin-window-state` owns SAVING (size/position/maximized on
//! move/resize/close, written on exit). It's told `skip_initial_state("main")`,
//! so RESTORE lives here: the plugin re-applies a saved size as raw *physical*
//! pixels with no scale adjustment, so on a mixed-DPI multi-monitor setup a
//! window sized on a 1x display reopens half-size (and below the min → OS-clamped
//! to the minimum) on a 2x display, or oversized the other way.
//!
//! We restore verbatim when the saved rect still lands on a connected monitor at
//! a compatible DPI, else open a comfortable default sized to the target monitor.
//!
//! Position is applied in *physical* pixels (absolute virtual-desktop coords, so
//! it's unambiguous across monitors). Size is reasoned about in physical pixels
//! for the on-screen/centering math but APPLIED in *logical* points: macOS sets
//! points directly, so a cross-monitor move can't mis-scale the window — whereas
//! `set_size(Physical)` converts via the window's current (possibly stale,
//! mid-move) scale factor and halves/doubles the result.
//!
//! One geometry slot per window (the plugin's model): last-close-wins, so
//! per-monitor geometry isn't remembered across unplug/replug — a good default
//! rather than the exact prior size in that case.

use std::collections::HashMap;

use tauri::{AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition};

// Fallbacks if the "main" window config can't be read. Mirror tauri.conf.json.
const DEFAULT_W: f64 = 1280.0;
const DEFAULT_H: f64 = 800.0;
const MIN_W: f64 = 1180.0;
const MIN_H: f64 = 620.0;

// The default window fills this fraction of the monitor's work area, so it scales
// up on large/non-retina displays instead of stranding a laptop-sized window on a
// big screen. Bounded below by the config default (keeps the laptop comfortable)
// and above by a cap (keeps it sane on 4K/ultrawide), and never within a sliver of
// filling the whole screen.
const DEFAULT_FRACTION: f64 = 0.75;
const MAX_DEFAULT_W: f64 = 2560.0;
const MAX_DEFAULT_H: f64 = 1600.0;
const MAX_WORK_FRACTION: f64 = 0.94;

/// Saved geometry as written by tauri-plugin-window-state (physical px). Unknown
/// fields are ignored; any missing field (or a parse failure) defaults to 0.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(default)]
struct SavedWindow {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    // Pre-maximize origin: when maximized, x/y hold the monitor's top-left.
    prev_x: i32,
    prev_y: i32,
    maximized: bool,
}

/// A monitor reduced to the numbers `decide` needs — full rect + usable work
/// area (physical, virtual-desktop coords) and the scale factor.
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
        // Some platforms under-report the work area as 0×0; fall back to full size.
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

/// The chosen geometry: physical top-left position + LOGICAL size (points). Two
/// variants so tests can assert which path ran; both are applied identically.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Plan {
    /// Saved monitor still present at a compatible DPI — restore verbatim.
    RestoreExact { x: i32, y: i32, w: f64, h: f64 },
    /// No trustworthy saved geometry — comfortable default, centered.
    DefaultCentered { x: i32, y: i32, w: f64, h: f64 },
}

impl Plan {
    fn rect(self) -> (i32, i32, f64, f64) {
        match self {
            Plan::RestoreExact { x, y, w, h } | Plan::DefaultCentered { x, y, w, h } => (x, y, w, h),
        }
    }
}

/// Best-effort restore of the main window's geometry; never blocks startup.
pub fn restore_main_window(app: &AppHandle) {
    if let Err(e) = try_restore(app) {
        log::warn!("main window geometry restore skipped: {e}");
    }
}

fn try_restore(app: &AppHandle) -> tauri::Result<()> {
    let Some(win) = app.get_webview_window("main") else {
        return Ok(());
    };

    // available_monitors() is reliable this early; current_monitor() can be None
    // before the window is placed (Linux/GTK), so it's only a default-path hint.
    let monitors: Vec<MonitorDesc> =
        win.available_monitors()?.iter().map(MonitorDesc::from_monitor).collect();
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

    log::info!(
        "window_geometry DIAG: saved={saved:?} current={current:?} monitors={monitors:?} plan={plan:?}"
    );

    // Position (absolute physical) moves the window onto the target monitor; the
    // size is applied in logical points so it can't be mis-scaled by the current
    // (possibly mid-move) monitor — see the module docs.
    let (x, y, w, h) = plan.rect();
    win.set_position(PhysicalPosition { x, y })?;
    win.set_size(LogicalSize { width: w, height: h })?;

    // The frame above is the pre-maximize ("unmaximize") target.
    if saved.map(|s| s.maximized).unwrap_or(false) {
        win.maximize()?;
    }
    Ok(())
}

/// (min_w, min_h, def_w, def_h) in logical px from the "main" window config.
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

/// Read the plugin's saved state, tolerating a missing/corrupt file. Mirrors the
/// plugin's own "ignore the all-zero default" guard.
fn load_saved(app: &AppHandle) -> Option<SavedWindow> {
    let path = app.path().app_config_dir().ok()?.join(tauri_plugin_window_state::DEFAULT_FILENAME);
    let raw = std::fs::read_to_string(path).ok()?;
    let mut map: HashMap<String, SavedWindow> = serde_json::from_str(&raw).ok()?;
    let s = map.remove("main")?;
    (s.width != 0 && s.height != 0).then_some(s)
}

/// Pure geometry decision, unit-tested below. Position is physical; size logical.
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
        // Maximized windows save the monitor's top-left in x/y; the real frame is
        // in prev_x/prev_y. Decide placement from the frame either way.
        let (px, py) = if s.maximized { (s.prev_x, s.prev_y) } else { (s.x, s.y) };
        if let Some(m) = monitors.iter().find(|m| intersects(m, px, py, s.width, s.height)) {
            // Trust the saved physical size only if it's still >= min at this
            // monitor's DPI. A low→high-DPI landing yields a sub-min logical size
            // the OS would clamp up to min (the "forgot my size" bug), so fall
            // back to a comfortable default there instead. Epsilon absorbs integer
            // rounding and fractional scale factors.
            let lw = s.width as f64 / m.scale;
            let lh = s.height as f64 / m.scale;
            if lw >= min_w - 1.0 && lh >= min_h - 1.0 {
                let (x, y, w, h) = clamp_to_work_area(px, py, s.width, s.height, m);
                // Physical size → logical points for the target monitor's DPI.
                return Plan::RestoreExact { x, y, w: w as f64 / m.scale, h: h as f64 / m.scale };
            }
            return default_on(m, min_w, min_h, def_w, def_h);
        }
    }
    default_on(&current, min_w, min_h, def_w, def_h)
}

/// Does the saved rect touch this monitor's full rect? Mirrors the plugin's
/// four-corner test (tauri-plugin-window-state lib.rs `MonitorExt::intersects`).
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

/// Shrink the rect to the monitor's work area and nudge it fully on-screen (all
/// physical px).
fn clamp_to_work_area(x: i32, y: i32, w: u32, h: u32, m: &MonitorDesc) -> (i32, i32, u32, u32) {
    let w = w.min(m.work_w);
    let h = h.min(m.work_h);
    let max_x = m.work_x + (m.work_w as i32 - w as i32);
    let max_y = m.work_y + (m.work_h as i32 - h as i32);
    let x = x.clamp(m.work_x, max_x.max(m.work_x));
    let y = y.clamp(m.work_y, max_y.max(m.work_y));
    (x, y, w, h)
}

/// A comfortable default centered on `m`, scaled to the monitor (see the
/// DEFAULT_* constants). Size is logical (points); position is the physical
/// centering of that size's physical footprint on `m`.
fn default_on(m: &MonitorDesc, min_w: f64, min_h: f64, def_w: f64, def_h: f64) -> Plan {
    let w_l = default_axis(m.work_w as f64 / m.scale, min_w, def_w, MAX_DEFAULT_W);
    let h_l = default_axis(m.work_h as f64 / m.scale, min_h, def_h, MAX_DEFAULT_H);
    // Center the physical footprint (logical × scale) within the work area.
    let w_px = (w_l * m.scale).round() as i32;
    let h_px = (h_l * m.scale).round() as i32;
    let x = m.work_x + (m.work_w as i32 - w_px) / 2;
    let y = m.work_y + (m.work_h as i32 - h_px) / 2;
    Plan::DefaultCentered { x, y, w: w_l, h: h_l }
}

/// One axis of the default size (logical): a fraction of the work area, at least
/// the config default, capped, and never within a sliver of filling the screen.
fn default_axis(work: f64, min: f64, def: f64, cap: f64) -> f64 {
    (DEFAULT_FRACTION * work).max(def).min(cap).min(MAX_WORK_FRACTION * work).max(min)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Full-rect == work-area monitor at the given origin/size/scale.
    fn mon(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorDesc {
        MonitorDesc { x, y, w, h, work_x: x, work_y: y, work_w: w, work_h: h, scale }
    }

    fn saved(x: i32, y: i32, w: u32, h: u32) -> SavedWindow {
        SavedWindow { width: w, height: h, x, y, prev_x: x, prev_y: y, maximized: false }
    }

    // Standard config dims for tests.
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
            Plan::RestoreExact { x: 200, y: 150, w: 1400.0, h: 900.0 }
        );
    }

    #[test]
    fn restore_size_is_logical_on_a_2x_monitor() {
        // Saved 2400x1600 physical on a 2x display → 1200x800 logical points.
        let m = mon(0, 0, 3024, 1964, 2.0);
        let s = saved(100, 100, 2400, 1600);
        assert_eq!(
            decide_std(Some(s), &[m], m),
            Plan::RestoreExact { x: 100, y: 100, w: 1200.0, h: 800.0 }
        );
    }

    #[test]
    fn off_screen_saved_falls_back_to_centered_default_on_current() {
        // External at negative coords is gone; only the 2x built-in remains.
        let built_in = mon(0, 0, 3024, 1964, 2.0);
        let s = saved(-3249, -888, 1934, 1109); // the real bug report's numbers
        // Default 1280x800 logical, centered (its 2560x1600 physical footprint fits).
        assert_eq!(
            decide_std(Some(s), &[built_in], built_in),
            Plan::DefaultCentered { x: 232, y: 182, w: 1280.0, h: 800.0 }
        );
    }

    #[test]
    fn sub_min_at_target_dpi_falls_back_to_default() {
        // Saved rect's coords overlap a surviving 2x monitor, but the saved
        // physical size is small → 1200/2 = 600 logical < 1180 min → reject.
        let m = mon(0, 0, 1920, 1080, 2.0);
        let s = saved(100, 100, 1200, 700);
        assert!(matches!(decide_std(Some(s), &[m], m), Plan::DefaultCentered { .. }));
    }

    #[test]
    fn oversized_saved_is_clamped_to_work_area() {
        let m = mon(0, 0, 1920, 1080, 1.0);
        let s = saved(0, 0, 4000, 3000); // bigger than the monitor
        assert_eq!(
            decide_std(Some(s), &[m], m),
            Plan::RestoreExact { x: 0, y: 0, w: 1920.0, h: 1080.0 }
        );
    }

    #[test]
    fn no_saved_state_scales_default_to_monitor() {
        let m = mon(0, 0, 1920, 1080, 1.0);
        // 0.75 * 1920 = 1440 wide, 0.75 * 1080 = 810 tall, centered.
        assert_eq!(
            decide_std(None, &[m], m),
            Plan::DefaultCentered { x: 240, y: 135, w: 1440.0, h: 810.0 }
        );
    }

    #[test]
    fn default_scales_up_on_large_non_retina_monitor() {
        // The reported case: a 1x ultrawide. 1280 logical looked tiny; the default
        // now fills 0.75 of the work area (2560, capped) instead.
        let m = mon(0, 0, 3440, 1440, 1.0);
        assert_eq!(
            decide_std(None, &[m], m),
            Plan::DefaultCentered { x: 440, y: 180, w: 2560.0, h: 1080.0 }
        );
    }

    #[test]
    fn picks_the_intersecting_monitor_among_several() {
        let left = mon(-3440, 0, 3440, 1440, 1.0); // 1x external, left of primary
        let primary = mon(0, 0, 3024, 1964, 2.0); // 2x built-in
        let s = saved(-3000, 200, 1934, 1109); // lives on the external
        assert_eq!(
            decide_std(Some(s), &[primary, left], primary),
            Plan::RestoreExact { x: -3000, y: 200, w: 1934.0, h: 1109.0 }
        );
    }

    #[test]
    fn default_stays_off_the_screen_edges_on_small_monitors() {
        // Config default (1280) would nearly fill a 1300-wide screen, so the 0.94
        // work-area guard pulls it in to 1222.
        let m = mon(0, 0, 1300, 900, 1.0);
        assert_eq!(
            decide_std(None, &[m], m),
            Plan::DefaultCentered { x: 39, y: 50, w: 1222.0, h: 800.0 }
        );
    }

    #[test]
    fn default_caps_at_max_on_huge_monitors() {
        let m = mon(0, 0, 5120, 2160, 1.0); // fraction ≫ cap
        let (_, _, w, h) = decide_std(None, &[m], m).rect();
        assert_eq!((w, h), (MAX_DEFAULT_W, MAX_DEFAULT_H));
    }

    #[test]
    fn maximized_uses_prev_coords_for_placement() {
        // Maximized: x/y are the monitor origin; prev_x/prev_y are the real frame.
        let external = mon(-3440, 0, 3440, 1440, 1.0);
        let primary = mon(0, 0, 3024, 1964, 2.0);
        let s = SavedWindow {
            width: 1600,
            height: 1000,
            x: -3440, // maximized origin (monitor top-left)
            y: 0,
            prev_x: -2000, // pre-maximize frame, on the external
            prev_y: 100,
            maximized: true,
        };
        // Placement decided from prev coords → external, restored verbatim.
        assert_eq!(
            decide_std(Some(s), &[primary, external], primary),
            Plan::RestoreExact { x: -2000, y: 100, w: 1600.0, h: 1000.0 }
        );
    }
}
