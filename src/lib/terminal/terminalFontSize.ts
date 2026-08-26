import { create } from "zustand";
import { persist } from "zustand/middleware";

const MIN = 8;
const MAX = 32;
const DEFAULT = 12;
const STEP = 1;

const clamp = (n: number): number => Math.max(MIN, Math.min(MAX, n));

type TerminalFontSizeState = {
  fontSize: number;
  zoomIn: () => void;
  zoomOut: () => void;
  reset: () => void;
};

export const useTerminalFontSize = create<TerminalFontSizeState>()(
  persist(
    (set) => ({
      fontSize: DEFAULT,
      zoomIn: () => set((s) => ({ fontSize: clamp(s.fontSize + STEP) })),
      zoomOut: () => set((s) => ({ fontSize: clamp(s.fontSize - STEP) })),
      reset: () => set({ fontSize: DEFAULT }),
    }),
    { name: "hyprlayer.terminalFontSize" },
  ),
);
