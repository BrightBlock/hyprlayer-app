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
      void queryClient.invalidateQueries({ queryKey: ["binaries"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
    }),
    // The thoughts index changed: docs were (re)indexed, removed, or the tree
    // was rescanned. Refresh every list, the touched documents (all of them on
    // a rescan, where ids is empty), and the repo git status (file changes move
    // the dirty count).
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
      void queryClient.invalidateQueries({ queryKey: ["thoughtsGitStatus"] });
    }),
  ];

  return () => {
    for (const p of unlisteners) {
      void p.then((unlisten) => unlisten());
    }
  };
}
