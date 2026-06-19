import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";

import { attachEventListeners } from "./lib/events";
import { queryClient } from "./lib/queryClient";
import { router } from "./router";
import { initTheme } from "./lib/theme";
import { useUpdate } from "./lib/updater";
import { setupNotificationActions } from "./lib/notifications";
import "./index.css";

// Apply the persisted color theme before the first paint to avoid a flash of
// the default palette.
initTheme();
attachEventListeners();
// Probe for a newer signed release on launch (and every 6h thereafter).
useUpdate.getState().init();
// Install the focus-edge router so a notification click (via app reactivation)
// jumps back to the workstream that fired it.
setupNotificationActions();

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
