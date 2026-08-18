import { create } from "zustand";

import type { PromptInput } from "./prompts";

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

/** A prompt run parked on its argument dialog. `id` keys the form so a second
 *  request remounts it empty instead of inheriting the first's typing. */
export type PromptArgsRequest = {
  id: number;
  name: string;
  inputs: PromptInput[];
  /** Pre-filled answers, text already typed after the composer's `/slug`. */
  initial: Record<string, string>;
  resolve: (values: Record<string, string> | null) => void;
};

export const usePromptArgsStore = create<{ request: PromptArgsRequest | null }>(() => ({
  request: null,
}));

let nextRequestId = 1;

/** Ask the user to fill a prompt's `{{input}}` slots; resolves to the values, or
 *  null when they cancel. A new ask cancels one already on screen, so a repeated
 *  chord can't leave the first run parked forever. */
export function askPromptArgs(
  name: string,
  inputs: PromptInput[],
  initial: Record<string, string> = {},
): Promise<Record<string, string> | null> {
  return new Promise((resolve) => {
    usePromptArgsStore.getState().request?.resolve(null);
    usePromptArgsStore.setState({
      request: { id: nextRequestId++, name, inputs, initial, resolve },
    });
  });
}

/** Settle the open argument request and close the dialog. */
export function answerPromptArgs(values: Record<string, string> | null): void {
  const req = usePromptArgsStore.getState().request;
  if (!req) return;
  usePromptArgsStore.setState({ request: null });
  req.resolve(values);
}
