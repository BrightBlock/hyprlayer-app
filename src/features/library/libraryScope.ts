
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

import type { LibrarySearch } from "./librarySearch";

type LibraryScopeStore = {
  lastProject: string | null;
  setLastProject: (project: string) => void;
  lastSearch: LibrarySearch | null;
  setLastSearch: (search: LibrarySearch) => void;
  lastSearchByProject: Record<string, LibrarySearch>;
  setProjectSearch: (project: string, search: LibrarySearch) => void;
  // Thoughts project names.
  pinnedProjects: string[];
  togglePinnedProject: (name: string) => void;
};

export const useLibraryScope = create<LibraryScopeStore>()(
  persist(
    (set) => ({
      lastProject: null,
      setLastProject: (project) => set({ lastProject: project }),
      lastSearch: null,
      setLastSearch: (search) => set({ lastSearch: search }),
      lastSearchByProject: {},
      setProjectSearch: (project, search) =>
        set((s) => ({
          lastSearchByProject: { ...s.lastSearchByProject, [project]: search },
        })),
      pinnedProjects: [],
      togglePinnedProject: (name) =>
        set((s) => ({
          pinnedProjects: s.pinnedProjects.includes(name)
            ? s.pinnedProjects.filter((p) => p !== name)
            : [...s.pinnedProjects, name],
        })),
    }),
    { name: scopedKey("hyprlayer:library-scope"), version: 1 },
  ),
);
