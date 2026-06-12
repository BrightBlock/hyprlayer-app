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
  ];

  return () => {
    for (const p of unlisteners) {
      void p.then((unlisten) => unlisten());
    }
  };
}
