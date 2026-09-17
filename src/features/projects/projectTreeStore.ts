
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

type ProjectTreeStore = {
  collapsedPaths: string[];
  toggleCollapsed: (path: string) => void;
};

export function toggleCollapsedPath(paths: string[], path: string): string[] {
  return paths.includes(path) ? paths.filter((p) => p !== path) : [...paths, path];
}

export const useProjectTreeStore = create<ProjectTreeStore>()(
  persist(
    (set) => ({
      collapsedPaths: [],
      toggleCollapsed: (path) =>
        set((s) => ({ collapsedPaths: toggleCollapsedPath(s.collapsedPaths, path) })),
    }),
    { name: scopedKey("hyprlayer:project-tree"), version: 1 },
  ),
);
