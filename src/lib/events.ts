import { events } from "./bindings";
import { queryClient } from "./queryClient";
import { invalidateScriptOutput, invalidateScriptRuntime, invalidateWorktreeGit } from "./queries";

/**
 * A feature-owned set of event subscriptions, attached/detached together with
 * the lib bridge below. Features register theirs from main.tsx (the wiring
 * layer), so this module never imports from src/features and the import
 * direction stays routes → features → lib (docs/adr/0003-frontend-layering.md).
 */
export type EventBridge = () => Array<Promise<() => void>>;

const featureBridges: EventBridge[] = [];

export function registerEventBridge(bridge: EventBridge): void {
  featureBridges.push(bridge);
}

/**
 * Bridge backend push events into query invalidation. Call once at startup;
 * returns a cleanup that detaches all listeners (useful for HMR).
 */
export function attachEventListeners(): () => void {
  const unlisteners: Promise<() => void>[] = [
    ...featureBridges.flatMap((bridge) => bridge()),
    // The hyprlayer CLI rewrote its config: projects/backends may differ.
    events.configChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
    }),
    // Binary probes changed (PATH enrichment, overrides, redetect). Project
    // detail depends on binaries too — it shells out to `hyprlayer`.
    events.binariesChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
    }),
    // A workstream's PR status changed server-side (merged, checks settled, a
    // PR appeared/disappeared). The backend watcher already diffed it, so just
    // refetch that workstream's PR query — only the mounted one (active
    // workstream / archive-on-merge) actually re-runs `gh`.
    events.prStatusChanged.listen((event) => {
      const { workstreamId } = event.payload;
      void queryClient.invalidateQueries({ queryKey: ["workstream", workstreamId, "pr"] });
    }),
    // The active workstream's worktree changed on disk (an external edit, a
    // build, a git checkout). The backend watches it and pushed this, so refresh
    // that workstream's git status + Changes panel live — no window focus needed.
    events.worktreeChanged.listen((event) => {
      invalidateWorktreeGit(event.payload.workstreamId);
    }),
    // A session started, changed state, or ended. Invalidate the active-sessions
    // list so the search palette's Agents section tracks running agents in real time.
    events.sessionLifecycle.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["active-sessions"] });
    }),
    // A lifecycle script (Setup/Run/Archive) started, detected its port, or
    // finished. Refresh that workstream's runtime (dock pills + Open :PORT).
    // The rail's live-Run tracking rides the same event via the scripts
    // feature's registered bridge (see scriptsStore.scriptRunBridge).
    events.scriptRunChanged.listen((event) => {
      const { workstreamId, kind } = event.payload;
      invalidateScriptRuntime(workstreamId);
      invalidateScriptOutput(workstreamId, kind);
    }),
    // The thoughts index changed: docs were (re)indexed, removed, or the tree
    // was rescanned. Refresh every list and the touched documents (all of
    // them on a rescan, where ids is empty).
    events.thoughtsIndexUpdated.listen((event) => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
      const { ids } = event.payload;
      if (ids.length === 0) {
        void queryClient.invalidateQueries({ queryKey: ["doc"] });
      } else {
        for (const id of ids) {
          void queryClient.invalidateQueries({ queryKey: ["doc", id] });
        }
      }
    }),
  ];

  return () => {
    for (const p of unlisteners) {
      void p.then((unlisten) => unlisten());
    }
  };
}
