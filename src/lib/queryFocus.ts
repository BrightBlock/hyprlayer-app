import { focusManager } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function setupQueryFocus(): void {
  focusManager.setEventListener((onFocus) => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => onFocus(focused))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  });
}
