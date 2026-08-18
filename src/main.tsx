import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { commands } from "./lib/bindings";
import { attachEventListeners, registerEventBridge } from "./lib/events";
import { hydrateRunning, scriptRunBridge } from "./features/session/scriptsStore";
import { removeSessionsForWorkstream } from "./features/session/sessionStore";
import { unwrap } from "./lib/ipc";
import { onWorkstreamArchived } from "./lib/queries";
import { queryClient } from "./lib/queryClient";
import { setupDragRegion } from "./lib/dragRegion";
import { setupMotionGate } from "./lib/motion";
import { setupQueryFocus } from "./lib/queryFocus";
import { router } from "./router";
import { initTheme } from "./lib/theme";
import { useUpdate } from "./lib/updater";
import { setupNotificationActions } from "./lib/notifications";
import "./index.css";

// Apply the persisted color theme before the first paint to avoid a flash of
// the default palette.
initTheme();
// Feature-side event/mutation hooks register before the bridge attaches, so
// lib/{events,queries} stay feature-independent (ADR-0003 import direction).
registerEventBridge(scriptRunBridge);
onWorkstreamArchived(removeSessionsForWorkstream);
attachEventListeners();
// Seed the rail's green "Run alive" indicator from any runs the backend still
// holds (survives a frontend reload; empty after a full app restart).
void unwrap(commands.listRunningScripts())
  .then(hydrateRunning)
  .catch(() => {});
// Probe for a newer signed release on launch (and every 6h thereafter).
useUpdate.getState().init();
// Install the focus-edge router so a notification click (via app reactivation)
// jumps back to the workstream that fired it.
setupNotificationActions();
// Route React Query's focus refetch through Tauri's window-focus signal (the
// WKWebview doesn't emit the browser focus event on app switches).
setupQueryFocus();
// Stop the looping animations while the app is in the background, a WebKitGTK
// window keeps painting off-screen, and painting one is not cheap.
setupMotionGate();
// Linux only: replace Tauri's drag-on-mousedown title bar with one that waits
// for the pointer to travel first, so a click on the header stays a click.
setupDragRegion();

// Suppress the webview's native right-click menu app-wide so the app doesn't
// feel like a web page. Editable surfaces (inputs, textareas, the CodeMirror
// contenteditable) keep the native menu so copy/paste/spellcheck still work, as
// does anything that opts in with `data-native-menu`. The `instanceof Element`
// guard avoids throwing when the event targets a non-Element node.
const NATIVE_MENU_SELECTOR =
  "input, textarea, [contenteditable]:not([contenteditable='false']), [data-native-menu]";
window.addEventListener("contextmenu", (e) => {
  const target = e.target;
  if (target instanceof Element && target.closest(NATIVE_MENU_SELECTOR)) return;
  e.preventDefault();
  // The webview word-selects under the cursor when opening a context menu; with
  // the native menu suppressed that highlight would just linger, so clear it.
  window.getSelection()?.removeAllRanges();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </React.StrictMode>,
);

// The window starts hidden on Windows/Linux (visible:false) so users never see
// the webview's white pre-paint frame or the window-state geometry restore
// happening mid-view. Reveal it once the dark UI has actually painted (two rAFs
// = one composited frame), with a timeout backstop so a missed frame can never
// strand the window hidden. On macOS the window is already visible, so show()
// is a harmless no-op.
{
  let revealed = false;
  const reveal = () => {
    if (revealed) return;
    revealed = true;
    void getCurrentWindow().show().catch(() => {});
  };
  requestAnimationFrame(() => requestAnimationFrame(reveal));
  setTimeout(reveal, 2000);
}
