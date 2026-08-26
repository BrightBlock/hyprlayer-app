import { getCurrentWindow } from "@tauri-apps/api/window";

const BLURRED = "data-window-blurred";

export function setupMotionGate(): void {
  const apply = (focused: boolean) => {
    document.documentElement.toggleAttribute(BLURRED, !focused);
  };

  const win = getCurrentWindow();
  void win.isFocused().then(apply, () => apply(true));
  void win.onFocusChanged(({ payload: focused }) => apply(focused));
}
