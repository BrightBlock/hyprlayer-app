import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";
import type { PipelineView } from "./pipeline";

type PipelineViewStore = {
  view: PipelineView;
  setView: (view: PipelineView) => void;
};

export const usePipelineViewStore = create<PipelineViewStore>()(
  persist(
    (set) => ({
      view: "board",
      setView: (view) => set({ view }),
    }),
    { name: scopedKey("hyprlayer:pipeline-view"), version: 1 },
  ),
);
