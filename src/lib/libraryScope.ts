/**
 * The Library's last-used project scope and last view state. A per-user view
 * preference, so — like [`pinStore`] / the agent-tab strips — it lives on the
 * frontend and persists to localStorage.
 *
 * `lastProject` holds the thoughts-project KEY (what docs are indexed under),
 * updated whenever a project becomes current (a project route) or the Library is
 * scoped to one; it seeds the scope on the first-ever Library open.
 *
 * `lastSearch` holds the whole Library view — scope, filters, and the open
 * document + body mode — so returning via the nav rail resumes the tab where the
 * user left it instead of resetting to a bare project scope and making them
 * re-find the document. (`from`, which project's "Docs" button opened it, is
 * navigational provenance rather than view state and is deliberately omitted.)
 *
 * `lastSearchByProject` holds that same view keyed per project, so each project's
 * "Docs" affordance reopens to the doc last read in *that* project — a single
 * global slot can't remember two projects the user alternates between.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { LibrarySearch } from "../router";

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
