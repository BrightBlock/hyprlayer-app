import { events } from "./bindings";
import { queryClient } from "./queryClient";

/**
 * Bridge backend push events into query invalidation. Call once at startup;
 * returns a cleanup that detaches all listeners (useful for HMR).
 */
export function attachEventListeners(): () => void {
  const unlisteners: Promise<() => void>[] = [
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
    // A session started, changed state, or ended. Invalidate the active-sessions
    // list so the search palette's Agents section tracks running agents in real time.
    events.sessionLifecycle.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["active-sessions"] });
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
