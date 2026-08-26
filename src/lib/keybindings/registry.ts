
import { useEffect, useRef } from "react";
import { create } from "zustand";

type RegistryState = { handlers: Record<string, Array<() => void>> };

const useRegistry = create<RegistryState>(() => ({ handlers: {} }));

function register(id: string, handler: () => void): () => void {
  useRegistry.setState((s) => ({
    handlers: { ...s.handlers, [id]: [...(s.handlers[id] ?? []), handler] },
  }));
  return () =>
    useRegistry.setState((s) => {
      const remaining = (s.handlers[id] ?? []).filter((h) => h !== handler);
      if (remaining.length > 0) return { handlers: { ...s.handlers, [id]: remaining } };
      const handlers = { ...s.handlers };
      delete handlers[id];
      return { handlers };
    });
}

export function useCommand(id: string, handler: (() => void) | null): void {
  const ref = useRef(handler);
  ref.current = handler;
  const enabled = handler !== null;
  useEffect(() => {
    if (!enabled) return;
    return register(id, () => ref.current?.());
  }, [id, enabled]);
}

export const ESCAPE_COMMAND_ID = "ui.escape";

export function useEscape(handler: (() => void) | null): void {
  useCommand(ESCAPE_COMMAND_ID, handler);
}

export function runCommand(id: string): boolean {
  const stack = useRegistry.getState().handlers[id];
  const handler = stack?.[stack.length - 1];
  if (!handler) return false;
  handler();
  return true;
}

export function isRegistered(id: string): boolean {
  return (useRegistry.getState().handlers[id]?.length ?? 0) > 0;
}
