import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { IS_MACOS } from "./platform";

/**
 * Tracks whether the window is maximized.
 *
 * Read off macOS, where the native title bar is removed (`decorations: false`)
 * and the app draws its own chrome: `WindowControls` swaps its maximize glyph
 * for a restore glyph, and `ResizeGrips` unmounts (a maximized window has no
 * edge to drag). macOS keeps its native overlay title bar, so this stays
 * `false` there and the hook does no work.
 */
export function useIsMaximized(): boolean {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (IS_MACOS) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    const sync = () =>
      void win.isMaximized().then((value) => {
        if (!cancelled) setMaximized(value);
      });

    sync();
    // Maximize/restore (incl. snap and double-click on the drag region) all
    // surface as a resize, so one listener keeps the glyph in sync.
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

  return maximized;
}
