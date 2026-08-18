/**
 * Pinned worktrees. The user can pin any active workstream so it surfaces in a
 * dedicated "Pinned" section at the top of the project sidebar, reachable
 * without hunting through its project's tree. This is a per-user *view*
 * preference (not project data), so it lives entirely on the frontend and
 * persists to localStorage, mirroring how the agent-tab strips survive a
 * restart (see [`workstreamTabs`]).
 *
 * A pin is keyed by the workstream's stable `id`. An id whose workstream is
 * archived or deleted simply stops resolving and drops out of the section (see
 * `selectPinnedViews`), so we never have to actively prune the list: a restored
 * workstream re-appears pinned, and a permanently deleted one is harmless dead
 * weight (a short string) that never renders.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { WorkstreamView } from "@/lib/bindings";

type PinStore = {
  /** Pinned workstream ids, in the order they were pinned (oldest first). */
  pinnedIds: string[];
  /** Whether the sidebar's Pinned section is collapsed (header chevron). */
  collapsed: boolean;
  /** Pin `id` if it isn't pinned, unpin it if it is. */
  togglePin: (id: string) => void;
  /** Collapse / expand the Pinned section. */
  toggleCollapsed: () => void;
};

/** Pure toggle: append `id` when missing, drop it when present. The remaining
 *  ids keep their order so unpinning one entry doesn't reshuffle the section. */
export function togglePinId(ids: string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
}

/** Resolve pinned ids to live workstream views, preserving pin order and
 *  silently dropping ids that no longer resolve (archived / deleted worktrees
 *  aren't in the active list). */
export function selectPinnedViews(
  pinnedIds: string[],
  views: WorkstreamView[],
): WorkstreamView[] {
  const byId = new Map(views.map((v) => [v.item.id, v]));
  const resolved: WorkstreamView[] = [];
  for (const id of pinnedIds) {
    const view = byId.get(id);
    if (view) resolved.push(view);
  }
  return resolved;
}

export const usePinStore = create<PinStore>()(
  persist(
    (set) => ({
      pinnedIds: [],
      collapsed: false,
      togglePin: (id) => set((s) => ({ pinnedIds: togglePinId(s.pinnedIds, id) })),
      toggleCollapsed: () => set((s) => ({ collapsed: !s.collapsed })),
    }),
    { name: "hyprlayer:pinned-workstreams", version: 1 },
  ),
);
