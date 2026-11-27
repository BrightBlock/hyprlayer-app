import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { commands } from "./lib/bindings";
import { wantsNativeMenu } from "./lib/platform";
import { attachEventListeners, onEventsMissed, onServerReconnected, registerEventBridge } from "./lib/events";
import { hydrateRunning, scriptRunBridge } from "./features/session/scriptsStore";
import { reconnectBridge } from "./features/hosts/reconnect";
import { sshPromptBridge } from "./features/hosts/sshPrompts";
import { startServerAutoUpdate } from "./features/hosts/serverAutoUpdate";
import { registerActiveWorkstreams, writtenDocsBridge } from "./features/session/writtenDocs";
import {
  activeWorkstreamIds,
  openGoalSession,
  reattachSessions,
  removeSessionsForWorkstream,
  sessionBridge,
} from "./features/session/sessionStore";
import { goalBridge, reconcileGoalBriefs } from "./features/session/goalBridge";
import { agentControlBridge, reconcileAgentTabs } from "./features/session/agentControlBridge";
import { leadNoteBridge } from "./features/lead/lead";
import { ptyAgentBridge, reconcilePtyAgents } from "./features/multiplexer/ptyAgentBridge";
import { reconcileAgentTerminals } from "./features/multiplexer/agentRuntime";
import {
  scheduleWatch,
  setupTranscriptSync,
  transcriptSyncBridge,
  transcriptSyncReconnected,
} from "./features/session/syncStore";
import { registerGoalHandoff } from "./features/session/goalHandoffSlot";
import { registerPlanCard } from "./features/session/planCardSlot";
import { registerReadyCard } from "./features/session/readyCardSlot";
import { registerReviewCard } from "./features/session/reviewCardSlot";
import { registerSendInterceptor } from "./features/session/sendInterceptor";
import { startSessionTitleSync } from "./features/session/sessionTitleSync";
import { PlanCard } from "./features/plan/PlanCard";
import { ReviewCard } from "./features/review/ReviewCard";
import { ReadyForReviewCard } from "./features/review/ReadyForReviewCard";
import { GoalHandoff } from "./features/review/GoalHandoff";
import { LifecycleSteps } from "./features/review/LifecycleSteps";
import { registerLifecycleSteps } from "./features/plan/lifecycleSlot";
import { approveFromChat } from "./features/plan/planApproval";
import { unwrap } from "./lib/ipc";
import { installGlobalHandlers, log } from "./lib/log";
import { invalidateGoals, onWorkstreamArchived } from "./lib/queries";
import { onGoalStarted } from "./lib/goalSheet";
import { queryClient } from "./lib/queryClient";
import { setupDragRegion } from "./lib/dragRegion";
import { setupMotionGate } from "./lib/motion";
import { setupQueryFocus } from "./lib/queryFocus";
import { router } from "./router";
import { initAppearance } from "./lib/appearance";
import { initTheme } from "./lib/theme";
import { syncTerminalColorScheme } from "./lib/terminal/terminalChrome";
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
syncTerminalColorScheme();
initAppearance();
registerEventBridge(scriptRunBridge);
registerEventBridge(writtenDocsBridge);
registerEventBridge(reconnectBridge);
registerEventBridge(sshPromptBridge);
registerEventBridge(sessionBridge);
registerEventBridge(transcriptSyncBridge);
registerEventBridge(goalBridge);
registerEventBridge(ptyAgentBridge);
registerEventBridge(agentControlBridge);
registerEventBridge(leadNoteBridge);
// Ahead of the reattach, whose folds ask whether the new connection syncs.
onServerReconnected(transcriptSyncReconnected);
onServerReconnected(() => {
  reattachSessions().catch((e) => log.warn("reconnect", "reattaching sessions failed", e));
});
// No goal event replays across a reconnect.
onServerReconnected(() => invalidateGoals());
onServerReconnected(() => {
  reconcilePtyAgents().catch((e) => log.warn("reconnect", "PTY agent states did not load", e));
});
onServerReconnected(() => {
  reconcileAgentTerminals().catch((e) => log.warn("reconnect", "PTY terminals were not checked", e));
});
onServerReconnected(() => {
  reconcileAgentTabs().catch((e) => log.warn("reconnect", "agent-spawned tabs did not load", e));
});
// What a reconnect re-reads, minus the session reattach: those streams are intact.
onEventsMissed(scheduleWatch);
onEventsMissed(() => {
  reconcileGoalBriefs().catch((e) => log.warn("events", "goal briefs did not reload", e));
});
onEventsMissed(() => {
  reconcilePtyAgents().catch((e) => log.warn("events", "PTY agent states did not load", e));
});
// A hooked agent's exit arrives only as `PtyAgentChanged`.
onEventsMissed(() => {
  reconcileAgentTerminals({ hookedOnly: true }).catch((e) => log.warn("events", "PTY terminals were not checked", e));
});
onEventsMissed(() => {
  reconcileAgentTabs().catch((e) => log.warn("events", "agent-spawned tabs did not load", e));
});
registerActiveWorkstreams(activeWorkstreamIds);
registerPlanCard(PlanCard);
registerReviewCard(ReviewCard);
registerReadyCard(ReadyForReviewCard);
registerGoalHandoff(GoalHandoff);
registerLifecycleSteps(LifecycleSteps);
registerSendInterceptor(approveFromChat);
onGoalStarted((goal, request) => {
  openGoalSession(goal, request).catch((e) => log.warn("goal", "opening the goal's session failed", e));
});
onWorkstreamArchived(removeSessionsForWorkstream);
attachEventListeners();
void unwrap(commands.listRunningScripts())
  .then(hydrateRunning)
  .catch((e) => log.warn("startup", "running scripts did not load", e));
reconcilePtyAgents().catch((e) => log.warn("startup", "PTY agent states did not load", e));
reconcileAgentTabs().catch((e) => log.warn("startup", "agent-spawned tabs did not load", e));
useUpdate.getState().init();
void startServerAutoUpdate();
setupNotificationActions();
startSessionTitleSync();
setupTranscriptSync();
setupQueryFocus();
setupMotionGate();
setupDragRegion();

window.addEventListener("contextmenu", (e) => {
  if (wantsNativeMenu(e.target)) return;
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

if (import.meta.env.VITE_PERF_PROBE) {
  void import("./perfWiring").then((m) => m.startPerf());
}

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
