import { focusManager } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Drive React Query's focus detection from Tauri's window-focus signal.
 *
 * React Query's default focus listener uses the browser `visibilitychange`
 * event, which a macOS WKWebview does NOT emit on app-to-app focus switches
 * (only on minimize/occlusion). So `refetchOnWindowFocus`, which the app opts
 * into globally, silently misses the common "tab away, change something on
 * disk, tab back" case. Tauri's `onFocusChanged` fires on every focus edge (the
 * same reliable signal the notification router uses), so wiring it here makes
 * focus refetch actually work. Concretely: create a missing worktree-copy file
 * on disk, return to the app, and the project's "copy files not found" warning
 * re-checks and clears without a restart.
 *
 * Replaces (not augments) the default listener, so there's no double-fire. Call
 * once at startup.
 */
export function setupQueryFocus(): void {
  focusManager.setEventListener((onFocus) => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => onFocus(focused))
      .then((fn) => {
        // Cleaned up before the async listener attached, detach immediately.
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  });
}
