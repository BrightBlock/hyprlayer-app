import { useEffect } from "react";
import { create } from "zustand";

import { commands } from "@/lib/bindings";

type ModelWindowsState = {
  windows: ReadonlyMap<string, number>;
  load: () => Promise<void>;
};

let attempted = false;

export const useModelWindows = create<ModelWindowsState>((set) => ({
  windows: new Map(),
  load: async () => {
    if (attempted) return;
    attempted = true;
    const res = await commands.claudeModelWindows();
    if (res.status === "ok") {
      set({ windows: new Map(res.data.map((m) => [m.id, m.contextWindow])) });
    }
  },
}));

export function useModelWindowMap(): ReadonlyMap<string, number> {
  const windows = useModelWindows((s) => s.windows);
  useEffect(() => {
    void useModelWindows.getState().load();
  }, []);
  return windows;
}
