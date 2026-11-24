import { getCurrentWindow } from "@tauri-apps/api/window";

declare global {
  interface Window {
    __HYPRLAYER_STILL_MOTION__?: boolean;
  }
}

const BLURRED = "data-window-blurred";
// Set by the shell when WebKitGTK runs without its DMA-BUF renderer, where every animated frame
// repaints the whole window on the webview's main thread. index.css holds endless indicators still.
const STILL = "data-still-motion";

export function setupMotionGate(): void {
  document.documentElement.toggleAttribute(STILL, window.__HYPRLAYER_STILL_MOTION__ === true);

  const apply = (focused: boolean) => {
    document.documentElement.toggleAttribute(BLURRED, !focused);
  };

  const win = getCurrentWindow();
  void win.isFocused().then(apply, () => apply(true));
  void win.onFocusChanged(({ payload: focused }) => apply(focused));
}
