
import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { WorkstreamView } from "@/lib/bindings";

type PinStore = {
  pinnedIds: string[];
  collapsed: boolean;
  togglePin: (id: string) => void;
  toggleCollapsed: () => void;
};

export function togglePinId(ids: string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
}

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
