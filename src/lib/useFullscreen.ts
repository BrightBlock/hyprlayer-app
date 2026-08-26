import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { IS_MACOS } from "./platform";

export function useIsFullscreen(): boolean {
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    if (!IS_MACOS) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    const sync = () =>
      void win.isFullscreen().then((value) => {
        if (!cancelled) setFullscreen(value);
      });

    sync();
    void win.onResized(sync).then((fn) => {
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
