/**
 * The Library's last-used project scope. Opening the Library defaults to the
 * most recently selected project (the most relevant one) rather than the
 * firehose of every project's docs; the switcher's "All projects" is then an
 * explicit, transient filter that doesn't overwrite this. A per-user view
 * preference, so — like [`pinStore`] / the agent-tab strips — it lives on the
 * frontend and persists to localStorage.
 *
 * Holds the thoughts-project KEY (what docs are indexed under), updated whenever
 * a project becomes current (a project route) or the Library is scoped to one.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

type LibraryScopeStore = {
  /** Thoughts-project key of the most recently selected project, or null. */
  lastProject: string | null;
  setLastProject: (project: string) => void;
};

export const useLibraryScope = create<LibraryScopeStore>()(
  persist(
    (set) => ({
      lastProject: null,
      setLastProject: (project) => set({ lastProject: project }),
    }),
    { name: "hyprlayer:library-scope", version: 1 },
  ),
);
