import { getCurrentWindow } from "@tauri-apps/api/window";

/** Set on `<html>` while the app is in the background; CSS pauses looping
 *  animations under it (see `index.css`). */
const BLURRED = "data-window-blurred";

/**
 * Pause the app's looping animations while the window is in the background.
 *
 * A WebKitGTK window keeps painting when it is not on screen: the Page
 * Visibility API stays `"visible"` for a window on another workspace, so none of
 * the throttling a browser would normally apply ever engages. An idle window on
 * another workspace measured the same CPU as a focused one.
 *
 * That would be cheap if a running animation were, but it isn't — WebKit
 * repaints the whole surface per frame here, and on a 4K 144 Hz output the
 * dashboard's single `kg-pulse` ring costs a full CPU core (2.9% with it
 * paused). Layer promotion doesn't help: `will-change`, an opacity-only
 * keyframe, and the same ring as a composited HTML element all still cost
 * 50-110% of a core. The only reliable lever is not animating.
 *
 * `visibilitychange` is not that lever (it never fires), and neither are the DOM
 * `focus`/`blur` events (they don't fire on a workspace switch). Tauri's
 * `onFocusChanged` does — the same signal `queryFocus` and the notification
 * router already rely on. Call once at startup.
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
