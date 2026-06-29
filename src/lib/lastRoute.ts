/**
 * The last app view (full route href, including search params), so a cold start
 * restores where the user left off — the workstream/project/library view they
 * had open — instead of always landing on the dashboard. A per-user view
 * preference, so — like [`libraryScope`] / the agent-tab strips — it lives on
 * the frontend and persists to localStorage. The view's data (open tabs, drafts)
 * already persists in its own stores; this just restores which view to show.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

type LastRouteStore = {
  /** Full href (pathname + search) of the last non-onboarding view, or null. */
  href: string | null;
  remember: (href: string) => void;
};

export const useLastRoute = create<LastRouteStore>()(
  persist(
    (set) => ({
      href: null,
      remember: (href) => set({ href }),
    }),
    { name: "hyprlayer:last-route", version: 1 },
  ),
);
