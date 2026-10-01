import { create } from "zustand";
import { persist } from "zustand/middleware";

export const TERMINAL_FONT_MIN = 8;
export const TERMINAL_FONT_MAX = 32;
export const TERMINAL_FONT_DEFAULT = 12;
const STEP = 1;

const clamp = (n: number): number => Math.max(TERMINAL_FONT_MIN, Math.min(TERMINAL_FONT_MAX, n));

type TerminalFontSizeState = {
  fontSize: number;
  zoomIn: () => void;
  zoomOut: () => void;
  reset: () => void;
};

export const useTerminalFontSize = create<TerminalFontSizeState>()(
  persist(
    (set) => ({
      fontSize: TERMINAL_FONT_DEFAULT,
      zoomIn: () => set((s) => ({ fontSize: clamp(s.fontSize + STEP) })),
      zoomOut: () => set((s) => ({ fontSize: clamp(s.fontSize - STEP) })),
      reset: () => set({ fontSize: TERMINAL_FONT_DEFAULT }),
    }),
    { name: "hyprlayer.terminalFontSize" },
  ),
);
