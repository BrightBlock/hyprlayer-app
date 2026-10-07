
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

type LastRouteStore = {
  href: string | null;
  remember: (href: string) => void;
};

// v1 Library hrefs can hold the retired sidebar's type tab and status pick; the nav's boxes own those now.
function dropRetiredLibraryFilters(href: string | null): string | null {
  const at = href?.indexOf("?") ?? -1;
  if (!href?.startsWith("/library") || at < 0) return href;
  const kept = href
    .slice(at + 1)
    .split("&")
    .filter((param) => !/^(type|status)=/.test(param));
  return href.slice(0, at) + (kept.length > 0 ? `?${kept.join("&")}` : "");
}

export const useLastRoute = create<LastRouteStore>()(
  persist(
    (set) => ({
      href: null,
      remember: (href) => set({ href }),
    }),
    {
      name: scopedKey("hyprlayer:last-route"),
      version: 2,
      migrate: (persisted, version) => {
        const state = persisted as LastRouteStore;
        return version >= 2 ? state : { ...state, href: dropRetiredLibraryFilters(state.href) };
      },
    },
  ),
);
