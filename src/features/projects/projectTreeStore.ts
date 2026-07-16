/**
 * Collapsed projects in the sidebar tree. A project's workstreams fold away
 * under a disclosure chevron, the way VSCode's explorer folds a folder.
 *
 * This is a per-user *view* preference (not project data), so it lives entirely
 * on the frontend and persists to localStorage — mirroring the Pinned section's
 * own collapse flag (see [`pinStore`]).
 *
 * Keyed by project path, and stored as the set of *collapsed* paths rather than
 * expanded ones: a newly added project has no entry, so it renders expanded by
 * default, and a project that is removed leaves behind a dead string that never
 * resolves and never renders. Nothing has to be actively pruned.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

type ProjectTreeStore = {
  /** Paths of projects whose workstreams are folded away. */
  collapsedPaths: string[];
  /** Collapse `path` if it's expanded, expand it if it's collapsed. */
  toggleCollapsed: (path: string) => void;
};

/** Pure toggle: append `path` when missing, drop it when present. */
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
    { name: "hyprlayer:project-tree", version: 1 },
  ),
);
