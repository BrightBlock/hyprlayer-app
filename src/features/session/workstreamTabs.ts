/**
 * Per-workstream agent-tab strips, held module-side (not in ProjectPage's
 * component state) so a tab's binding to its session — plus its draft and
 * attached context docs — survives navigating away from the project page and
 * back. Persisted to localStorage so it also survives an app restart: the tab
 * keeps its `sessionId`, and on reopen the transcript is rebuilt from the
 * session's SQLite event log (see `restoreSessionIfNeeded` in [`sessionStore`]).
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { ShellTab } from "../../routes/ProjectPage";

/** One workstream's whole tab strip, updated atomically: its shells, the
 *  selected shell (which survives opening the Docs view — it's the attach
 *  target), and whether the Docs view is showing. */
export type WsTabState = {
  shells: ShellTab[];
  activeId: number;
  docsOpen: boolean;
};

export const defaultTabState = (): WsTabState => ({
  shells: [{ id: 1, title: "Agent", mode: "general", draft: "", docs: [] }],
  activeId: 1,
  docsOpen: false,
});

type TabsStore = { byWs: Record<string, WsTabState> };

export const useWorkstreamTabsStore = create<TabsStore>()(
  persist(() => ({ byWs: {} }), {
    name: "hyprlayer:workstream-tabs",
    version: 1,
  }),
);

/** Apply a functional update to the whole `{ wsId -> tab strip }` map. */
export function updateWorkstreamTabs(
  next: (prev: Record<string, WsTabState>) => Record<string, WsTabState>,
): void {
  useWorkstreamTabsStore.setState((s) => ({ byWs: next(s.byWs) }));
}

/**
 * Open (creating if absent) the dedicated Ship tab for a workstream, bind it to
 * `sessionId`, and select it. Shipping reuses one tab per workstream rather than
 * being a chosen chat mode (see `shipWorkstream` in [`sessionStore`]).
 */
export function openShipTab(workstreamId: string, sessionId: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    const existing = current.shells.find((s) => s.mode === "ship");
    if (existing) {
      return {
        ...all,
        [workstreamId]: {
          ...current,
          shells: current.shells.map((s) => (s.id === existing.id ? { ...s, sessionId } : s)),
          activeId: existing.id,
          docsOpen: false,
        },
      };
    }
    const nextId = Math.max(0, ...current.shells.map((s) => s.id)) + 1;
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          { id: nextId, title: "Ship", mode: "ship", draft: "", docs: [], sessionId },
        ],
        activeId: nextId,
        docsOpen: false,
      },
    };
  });
}
