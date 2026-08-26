import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { commands } from "./lib/bindings";
import { attachEventListeners, registerEventBridge } from "./lib/events";
import { hydrateRunning, scriptRunBridge } from "./features/session/scriptsStore";
import { registerActiveWorkstreams, writtenDocsBridge } from "./features/session/writtenDocs";
import { activeWorkstreamIds, removeSessionsForWorkstream } from "./features/session/sessionStore";
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

initTheme();
registerEventBridge(scriptRunBridge);
registerEventBridge(writtenDocsBridge);
registerActiveWorkstreams(activeWorkstreamIds);
onWorkstreamArchived(removeSessionsForWorkstream);
attachEventListeners();
void unwrap(commands.listRunningScripts())
  .then(hydrateRunning)
  .catch(() => {});
useUpdate.getState().init();
setupNotificationActions();
setupQueryFocus();
setupMotionGate();
setupDragRegion();

const NATIVE_MENU_SELECTOR =
  "input, textarea, [contenteditable]:not([contenteditable='false']), [data-native-menu]";
window.addEventListener("contextmenu", (e) => {
  const target = e.target;
  if (target instanceof Element && target.closest(NATIVE_MENU_SELECTOR)) return;
  e.preventDefault();
  window.getSelection()?.removeAllRanges();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </React.StrictMode>,
);

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
