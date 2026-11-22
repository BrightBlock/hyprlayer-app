import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

import type { WorkstreamView } from "@/lib/bindings";

type ProjectTreeStore = {
  collapsedPaths: string[];
  toggleCollapsed: (path: string) => void;
  collapsedParentIds: string[];
  toggleParentCollapsed: (id: string) => void;
};

export function toggleCollapsedPath(paths: string[], path: string): string[] {
  return paths.includes(path) ? paths.filter((p) => p !== path) : [...paths, path];
}

export type NestedWorkstream = { view: WorkstreamView; depth: number; childCount: number };

export function nestWorkstreams(
  views: WorkstreamView[],
  collapsedParentIds: string[],
): NestedWorkstream[] {
  const listed = new Set(views.map((view) => view.item.id));
  const children = new Map<string, WorkstreamView[]>();
  const roots: WorkstreamView[] = [];
  for (const view of views) {
    const parent = view.item.parentWorkstreamId;
    if (parent && parent !== view.item.id && listed.has(parent)) {
      children.set(parent, [...(children.get(parent) ?? []), view]);
    } else {
      roots.push(view);
    }
  }
  const rows: NestedWorkstream[] = [];
  const visit = (view: WorkstreamView, depth: number) => {
    const kids = children.get(view.item.id) ?? [];
    rows.push({ view, depth, childCount: kids.length });
    if (collapsedParentIds.includes(view.item.id)) return;
    for (const kid of kids) visit(kid, depth + 1);
  };
  for (const root of roots) visit(root, 0);
  return rows;
}

export const useProjectTreeStore = create<ProjectTreeStore>()(
  persist(
    (set) => ({
      collapsedPaths: [],
      toggleCollapsed: (path) =>
        set((s) => ({ collapsedPaths: toggleCollapsedPath(s.collapsedPaths, path) })),
      collapsedParentIds: [],
      toggleParentCollapsed: (id) =>
        set((s) => ({ collapsedParentIds: toggleCollapsedPath(s.collapsedParentIds, id) })),
    }),
    { name: scopedKey("hyprlayer:project-tree"), version: 1 },
  ),
);
