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

/** One workstream's whole tab strip, updated atomically.
 *
 *  A workstream can run a side-by-side split: each shell carries a `panel`
 *  ("main" | "right"), and the two panes track their own active shell —
 *  `activeId` for the main pane, `rightActiveId` for the right one. `rightSplit`
 *  is whether the right pane is open; `activePane` is the focused pane (only its
 *  active tab reads as fully selected). Mirrors termic's right-split model. */
export type WsTabState = {
  shells: ShellTab[];
  activeId: number;
  rightSplit?: boolean;
  rightActiveId?: number;
  activePane?: "main" | "right";
};

export const defaultTabState = (): WsTabState => ({
  shells: [
    { id: 1, title: "Agent", mode: "general", draft: "", docs: [], attachments: [], panel: "main" },
  ],
  activeId: 1,
});

/** Next free shell id for a workstream — ids only ever grow, so a tab's id is
 *  stable for its lifetime even as siblings come and go. */
export const nextShellId = (shells: ShellTab[]): number =>
  Math.max(0, ...shells.map((s) => s.id)) + 1;

type TabsStore = { byWs: Record<string, WsTabState> };

export const useWorkstreamTabsStore = create<TabsStore>()(
  persist(() => ({ byWs: {} }), {
    name: "hyprlayer:workstream-tabs",
    // v2 added `attachments` to every shell — backfill it on rehydrate so tabs
    // persisted by an older build don't crash on `shell.attachments.length`.
    version: 2,
    migrate: (persisted) => {
      const state = persisted as TabsStore | undefined;
      if (!state?.byWs) return { byWs: {} };
      for (const ws of Object.values(state.byWs)) {
        ws.shells = (ws.shells ?? []).map((s) => ({
          ...s,
          docs: s.docs ?? [],
          attachments: s.attachments ?? [],
        }));
      }
      return state;
    },
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
      const inRightPane = existing.panel === "right";
      return {
        ...all,
        [workstreamId]: {
          ...current,
          shells: current.shells.map((s) => (s.id === existing.id ? { ...s, sessionId } : s)),
          activeId: inRightPane ? current.activeId : existing.id,
          rightActiveId: inRightPane ? existing.id : current.rightActiveId,
          rightSplit: inRightPane ? true : current.rightSplit,
          activePane: inRightPane ? "right" : "main",
        },
      };
    }
    const nextId = nextShellId(current.shells);
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          {
            id: nextId,
            title: "Ship",
            mode: "ship",
            draft: "",
            docs: [],
            attachments: [],
            sessionId,
            panel: "main",
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
    };
  });
}
