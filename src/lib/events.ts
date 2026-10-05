import { events } from "./bindings";
import { queryClient } from "./queryClient";
import { refreshOmarchyTheme } from "./theme";
import {
  invalidateCliBundleState,
  invalidateReview,
  invalidateScriptOutput,
  invalidateScriptRuntime,
  invalidateWorkstreamLists,
  invalidateWorkstreamPr,
  invalidateWorktreeGit,
} from "./queries";

export type EventBridge = () => Array<Promise<() => void>>;

const featureBridges: EventBridge[] = [];

export function registerEventBridge(bridge: EventBridge): void {
  featureBridges.push(bridge);
}

const reconnectHandlers: Array<() => void> = [];

export function onServerReconnected(handler: () => void): void {
  reconnectHandlers.push(handler);
}

export function notifyServerReconnected(): void {
  for (const handler of reconnectHandlers) handler();
}

export function attachEventListeners(): () => void {
  const unlisteners: Promise<() => void>[] = [
    ...featureBridges.flatMap((bridge) => bridge()),
    events.configChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
      void queryClient.invalidateQueries({ queryKey: ["thoughts-profiles"] });
    }),
    events.binariesChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
      void queryClient.invalidateQueries({ queryKey: ["claude-probe"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
      invalidateCliBundleState();
    }),
    // The edge event tells the UI to re-read the level state.
    events.serverDisconnected.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["connection-status"] });
    }),
    events.omarchyThemeChanged.listen(() => {
      void refreshOmarchyTheme();
    }),
    events.prStatusChanged.listen((event) => {
      invalidateWorkstreamPr(event.payload.workstreamId);
    }),
    events.worktreeChanged.listen((event) => {
      invalidateWorktreeGit(event.payload.workstreamId);
    }),
    // Only mutations announce this; a read that did would refetch itself forever.
    events.reviewChanged.listen((event) => {
      invalidateReview(event.payload.workstreamId);
    }),
    events.workstreamBranchChanged.listen((event) => {
      invalidateWorkstreamLists();
      invalidateWorkstreamPr(event.payload.workstreamId);
    }),
    events.sessionLifecycle.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["active-sessions"] });
      void queryClient.invalidateQueries({ queryKey: ["workstream-sessions"] });
    }),
    events.scriptRunChanged.listen((event) => {
      const { workstreamId, kind } = event.payload;
      invalidateScriptRuntime(workstreamId);
      invalidateScriptOutput(workstreamId, kind);
    }),
    events.thoughtsIndexUpdated.listen((event) => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
      void queryClient.invalidateQueries({ queryKey: ["doc-counts"] });
      void queryClient.invalidateQueries({ queryKey: ["related"] });
      void queryClient.invalidateQueries({ queryKey: ["thoughts-git-status"] });
      const { ids } = event.payload;
      if (ids.length === 0) {
        void queryClient.invalidateQueries({ queryKey: ["doc"] });
      } else {
        for (const id of ids) {
          void queryClient.invalidateQueries({ queryKey: ["doc", id] });
        }
      }
    }),
    events.docStateChanged.listen((event) => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
      void queryClient.invalidateQueries({ queryKey: ["doc-counts"] });
      // Read toggles leave the bytes alone: an open doc isn't re-read, only refetched on its next open.
      const { ids, change } = event.payload;
      for (const id of ids) {
        void queryClient.invalidateQueries({
          queryKey: ["doc", id],
          refetchType: change === "agent" ? "active" : "none",
        });
      }
    }),
  ];

  return () => {
    for (const p of unlisteners) {
      void p.then((unlisten) => unlisten());
    }
  };
}
