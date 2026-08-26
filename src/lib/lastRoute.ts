
import { create } from "zustand";
import { persist } from "zustand/middleware";

type LastRouteStore = {
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
