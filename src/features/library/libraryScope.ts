/**
 * The Library's last-used project scope and view state. A per-user view
 * preference, so like [`pinStore`] and the agent-tab strips it lives on the
 * frontend and persists to localStorage.
 *
 * `lastProject` holds the thoughts-project KEY that docs are indexed under. It
 * updates whenever a project becomes current or the Library is scoped to one,
 * and seeds the scope on the first Library open.
 *
 * `lastSearch` holds the whole view: scope, filters, and the open document plus
 * body mode. Returning via the nav rail resumes where you left off instead of
 * resetting to a bare scope. (`from`, naming which project's "Docs" button
 * opened it, is provenance rather than view state, so it is omitted.)
 *
 * `lastSearchByProject` keys that same view per project, so each project's
 * "Docs" button reopens the doc last read in *that* project. One global slot
 * could not remember two projects you alternate between.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { LibrarySearch } from "./librarySearch";

type LibraryScopeStore = {
  /** Thoughts-project key of the most recently selected project, or null. */
  lastProject: string | null;
  setLastProject: (project: string) => void;
  /** The Library's last view state (scope + filters + open doc), or null. */
  lastSearch: LibrarySearch | null;
  setLastSearch: (search: LibrarySearch) => void;
  /** Last view per project (thoughts-project key → view), for its "Docs" entry. */
  lastSearchByProject: Record<string, LibrarySearch>;
  setProjectSearch: (project: string, search: LibrarySearch) => void;
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
    }),
    { name: "hyprlayer:library-scope", version: 1 },
  ),
);
