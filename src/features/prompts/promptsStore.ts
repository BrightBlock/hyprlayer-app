import { create } from "zustand";

/** Open/closed state of the prompt palette (⌘K), mirroring `searchStore`. */
export const usePromptPaletteStore = create<{ open: boolean }>(() => ({ open: false }));

export function openPromptPalette(): void {
  usePromptPaletteStore.setState({ open: true });
}

export function closePromptPalette(): void {
  usePromptPaletteStore.setState({ open: false });
}

export function togglePromptPalette(): void {
  usePromptPaletteStore.setState((s) => ({ open: !s.open }));
}
