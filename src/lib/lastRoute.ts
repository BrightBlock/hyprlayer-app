/**
 * The last app view, as a full route href including search params, so a cold
 * start reopens the workstream or library view you left instead of the
 * dashboard. A per-user view preference, so like [`libraryScope`] and the
 * agent-tab strips it lives on the frontend and persists to localStorage.
 *
 * The view's own data (open tabs, drafts) persists in its own stores. This
 * only restores which view to show.
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
