import { getCurrentWindow } from "@tauri-apps/api/window";

/** Set on `<html>` while the app is in the background; CSS pauses looping
 *  animations under it (see `index.css`). */
const BLURRED = "data-window-blurred";

/**
 * Pause the app's looping animations while the window is in the background.
 *
 * A WebKitGTK window keeps painting off screen. The Page Visibility API reports
 * `"visible"` for a window on another workspace, so none of a browser's usual
 * throttling engages: an idle window measured the same CPU as a focused one.
 *
 * Animation is the expensive part. WebKit repaints the whole surface per frame,
 * so on a 4K 144 Hz output the dashboard's single `kg-pulse` ring costs a full
 * core (2.9% with it paused). Layer promotion doesn't help: `will-change`, an
 * opacity-only keyframe, and the ring as a composited HTML element each still
 * cost 50-110% of a core. The only reliable lever is not animating.
 *
 * `visibilitychange` never fires here, and DOM `focus`/`blur` don't fire on a
 * workspace switch. Tauri's `onFocusChanged` does. Call once at startup.
 */
export function setupMotionGate(): void {
  const apply = (focused: boolean) => {
    document.documentElement.toggleAttribute(BLURRED, !focused);
  };

  const win = getCurrentWindow();
  // The window comes up focused; correct it if that is already stale.
  void win.isFocused().then(apply, () => apply(true));
  void win.onFocusChanged(({ payload: focused }) => apply(focused));
}
