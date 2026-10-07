import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { commands } from "./lib/bindings";
import { attachEventListeners, onServerReconnected, registerEventBridge } from "./lib/events";
import { hydrateRunning, scriptRunBridge } from "./features/session/scriptsStore";
import { reconnectBridge } from "./features/hosts/reconnect";
import { sshPromptBridge } from "./features/hosts/sshPrompts";
import { startServerAutoUpdate } from "./features/hosts/serverAutoUpdate";
import { registerActiveWorkstreams, writtenDocsBridge } from "./features/session/writtenDocs";
import {
  activeWorkstreamIds,
  reattachSessions,
  removeSessionsForWorkstream,
  sessionBridge,
} from "./features/session/sessionStore";
import {
  setupTranscriptSync,
  transcriptSyncBridge,
  transcriptSyncReconnected,
} from "./features/session/syncStore";
import { registerPlanCard } from "./features/session/planCardSlot";
import { registerReadyCard } from "./features/session/readyCardSlot";
import { registerReviewCard } from "./features/session/reviewCardSlot";
import { registerSendInterceptor } from "./features/session/sendInterceptor";
import { startSessionTitleSync } from "./features/session/sessionTitleSync";
import { PlanCard } from "./features/plan/PlanCard";
import { ReviewCard } from "./features/review/ReviewCard";
import { ReadyForReviewCard } from "./features/review/ReadyForReviewCard";
import { ReviewChecklist } from "./features/review/ReviewChecklist";
import { registerReviewChecklist } from "./features/plan/reviewChecklistSlot";
import { approveFromChat } from "./features/plan/planApproval";
import { unwrap } from "./lib/ipc";
import { installGlobalHandlers, log } from "./lib/log";
import { onWorkstreamArchived } from "./lib/queries";
import { queryClient } from "./lib/queryClient";
import { setupDragRegion } from "./lib/dragRegion";
import { setupMotionGate } from "./lib/motion";
import { setupQueryFocus } from "./lib/queryFocus";
import { router } from "./router";
import { initAppearance } from "./lib/appearance";
import { initTheme } from "./lib/theme";
import { useUpdate } from "./lib/updater";
import { setupNotificationActions } from "./lib/notifications";
import "@fontsource-variable/inter/wght.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "@fontsource-variable/geist/wght.css";
import "@fontsource-variable/ibm-plex-sans/wght.css";
import "@fontsource-variable/geist-mono/wght.css";
import "@fontsource-variable/fira-code/wght.css";
import "@fontsource/instrument-serif/400.css";
import "@fontsource/instrument-serif/400-italic.css";
import "./index.css";

installGlobalHandlers();
initTheme();
initAppearance();
registerEventBridge(scriptRunBridge);
registerEventBridge(writtenDocsBridge);
registerEventBridge(reconnectBridge);
registerEventBridge(sshPromptBridge);
registerEventBridge(sessionBridge);
registerEventBridge(transcriptSyncBridge);
// Ahead of the reattach, whose folds ask whether the new connection syncs.
onServerReconnected(transcriptSyncReconnected);
onServerReconnected(() => {
  reattachSessions().catch((e) => log.warn("reconnect", "reattaching sessions failed", e));
});
registerActiveWorkstreams(activeWorkstreamIds);
registerPlanCard(PlanCard);
registerReviewCard(ReviewCard);
registerReadyCard(ReadyForReviewCard);
registerReviewChecklist(ReviewChecklist);
registerSendInterceptor(approveFromChat);
onWorkstreamArchived(removeSessionsForWorkstream);
attachEventListeners();
void unwrap(commands.listRunningScripts())
  .then(hydrateRunning)
  .catch((e) => log.warn("startup", "running scripts did not load", e));
useUpdate.getState().init();
void startServerAutoUpdate();
setupNotificationActions();
startSessionTitleSync();
setupTranscriptSync();
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

ReactDOM.createRoot(document.getElementById("root") as HTMLElement, {
  onCaughtError: (error, info) =>
    log.error("render", `caught by an error boundary${info.componentStack ?? ""}`, error),
  onUncaughtError: (error, info) =>
    log.error("render", `uncaught${info.componentStack ?? ""}`, error),
  onRecoverableError: (error) => log.warn("render", "recovered", error),
}).render(
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
    // Windows and Linux start hidden: a failed show is an invisible app.
    void getCurrentWindow()
      .show()
      .catch((e) => log.error("startup", "could not show the window", e));
  };
  requestAnimationFrame(() => requestAnimationFrame(reveal));
  setTimeout(reveal, 2000);
}
