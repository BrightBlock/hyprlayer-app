import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { IS_MACOS } from "./platform";

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
    void win.onResized(sync).then((fn) => {
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
