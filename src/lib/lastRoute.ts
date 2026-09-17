
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

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
    { name: scopedKey("hyprlayer:last-route"), version: 1 },
  ),
);
