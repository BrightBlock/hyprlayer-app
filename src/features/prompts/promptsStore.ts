import { create } from "zustand";

import type { PromptInput } from "./prompts";

export const usePromptPaletteStore = create<{ open: boolean }>(() => ({ open: false }));

export function closePromptPalette(): void {
  usePromptPaletteStore.setState({ open: false });
}

export function togglePromptPalette(): void {
  usePromptPaletteStore.setState((s) => ({ open: !s.open }));
}

export type PromptArgsRequest = {
  id: number;
  name: string;
  inputs: PromptInput[];
  initial: Record<string, string>;
  resolve: (values: Record<string, string> | null) => void;
};

export const usePromptArgsStore = create<{ request: PromptArgsRequest | null }>(() => ({
  request: null,
}));

let nextRequestId = 1;

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

export function answerPromptArgs(values: Record<string, string> | null): void {
  const req = usePromptArgsStore.getState().request;
  if (!req) return;
  usePromptArgsStore.setState({ request: null });
  req.resolve(values);
}
