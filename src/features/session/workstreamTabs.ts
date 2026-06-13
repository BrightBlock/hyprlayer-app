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
