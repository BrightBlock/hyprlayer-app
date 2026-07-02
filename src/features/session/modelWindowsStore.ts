/**
 * Authoritative per-model context windows, fetched once from the Anthropic
 * Models API (`claude_model_windows` → `max_input_tokens`) reusing the user's
 * Claude login. The context meter prefers these over the hand-maintained table
 * in `models.ts`, which silently drifts when Anthropic ships or resizes a model
 * (Fable 5 shipped at 1M but was pinned to 200K).
 *
 * Best-effort by design: a failed fetch (offline, logged out, no API key, denied
 * Keychain access) leaves the map empty and the meter falls back to the static
 * table — so the app never depends on this succeeding. The fetch fires lazily on
 * first use (see [`useModelWindowMap`]) rather than at launch, so the one-time
 * macOS Keychain access prompt lands when a session actually needs the meter, not
 * on every cold start.
 */
import { useEffect } from "react";
import { create } from "zustand";

import { commands } from "../../lib/bindings";

type ModelWindowsState = {
  /** Canonical (bare, no `[1m]`) model id → context window in tokens. */
  windows: ReadonlyMap<string, number>;
  /** Fetch the catalog once per app session; a no-op after the first attempt. */
  load: () => Promise<void>;
};

// Module-scoped so the guard is shared across every hook subscriber: we attempt
// the fetch exactly once, whether it succeeds or fails. A failure leaves the
// static table in charge and isn't retried until the next launch (retrying per
// meter render would re-prompt the Keychain on every denial).
let attempted = false;

export const useModelWindows = create<ModelWindowsState>((set) => ({
  windows: new Map(),
  load: async () => {
    if (attempted) return;
    attempted = true;
    const res = await commands.claudeModelWindows();
    if (res.status === "ok") {
      set({ windows: new Map(res.data.map((m) => [m.id, m.contextWindow])) });
    }
    // On error: swallow. The static per-model table covers the meter.
  },
}));

/** Subscribe to the fetched windows, kicking off the one-shot load on first use. */
export function useModelWindowMap(): ReadonlyMap<string, number> {
  const windows = useModelWindows((s) => s.windows);
  useEffect(() => {
    void useModelWindows.getState().load();
  }, []);
  return windows;
}
