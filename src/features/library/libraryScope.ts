
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

// v1 searches can hold the retired sidebar's type tab and status pick; the nav's boxes own those now.
function dropRetiredFilters(search: LibrarySearch): LibrarySearch {
  return { ...search, type: undefined, status: undefined };
}

function migrateLibraryScope(persisted: unknown, version: number): LibraryScopeStore {
  const state = persisted as LibraryScopeStore;
  if (version >= 2) return state;
  return {
    ...state,
    lastSearch: state.lastSearch ? dropRetiredFilters(state.lastSearch) : null,
    lastSearchByProject: Object.fromEntries(
      Object.entries(state.lastSearchByProject ?? {}).map(([project, search]) => [project, dropRetiredFilters(search)]),
    ),
  };
}

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
    { name: scopedKey("hyprlayer:library-scope"), version: 2, migrate: migrateLibraryScope },
  ),
);
