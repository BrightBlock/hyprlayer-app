import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { IS_MACOS } from "./platform";

/**
 * Tracks whether the window is in native (green-button) fullscreen.
 *
 * Only macOS floats its window controls over the content (the overlay title bar)
 * and hides them on fullscreen, so the chrome insets that reserve space for the
 * traffic lights must collapse there. Off macOS this stays `false` and the hook
 * does no work (Windows/Linux put their controls elsewhere). Re-reads
 * `isFullscreen()` on every window resize — the signal a fullscreen toggle fires
 * — mirroring the Tauri-window-event pattern in `queryFocus.ts`.
 */
export function useIsFullscreen(): boolean {
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    if (!IS_MACOS) return; // only macOS hides its window controls on fullscreen
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    const sync = () =>
      void win.isFullscreen().then((value) => {
        if (!cancelled) setFullscreen(value);
      });

    sync(); // initial read — the window may launch already fullscreen
    void win.onResized(sync).then((fn) => {
      // Cleaned up before the async listener attached — detach immediately.
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return fullscreen;
}
