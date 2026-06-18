/**
 * Command registry: components register a handler for a command id while they're
 * mounted; the global dispatcher (`useKeybindings`) looks the id up and runs it.
 *
 * Whether a handler is currently registered IS the runtime context predicate —
 * e.g. *Stop turn* is only registered while a session is running, so the same
 * chord is simply inert elsewhere. A null handler means "not available right
 * now", so the dispatcher lets the keystroke pass through.
 */

import { useEffect, useRef } from "react";
import { create } from "zustand";

type RegistryState = { handlers: Record<string, () => void> };

const useRegistry = create<RegistryState>(() => ({ handlers: {} }));

function register(id: string, handler: () => void): void {
  useRegistry.setState((s) => ({ handlers: { ...s.handlers, [id]: handler } }));
}

function unregister(id: string): void {
  useRegistry.setState((s) => {
    if (!(id in s.handlers)) return s;
    const handlers = { ...s.handlers };
    delete handlers[id];
    return { handlers };
  });
}

/**
 * Register `handler` for command `id` while mounted; pass `null` to leave the
 * command unbound in this context (the gate). The latest closure always runs —
 * a ref absorbs render-to-render identity churn, so only `id` or the
 * enabled/disabled transition re-runs the effect.
 */
export function useCommand(id: string, handler: (() => void) | null): void {
  const ref = useRef(handler);
  ref.current = handler;
  const enabled = handler !== null;
  useEffect(() => {
    if (!enabled) return;
    register(id, () => ref.current?.());
    return () => unregister(id);
  }, [id, enabled]);
}

/** Run the handler registered for `id`. Returns true iff one was registered. */
export function runCommand(id: string): boolean {
  const handler = useRegistry.getState().handlers[id];
  if (!handler) return false;
  handler();
  return true;
}

/** Test-only: whether a handler is currently registered for `id`. */
export function isRegistered(id: string): boolean {
  return id in useRegistry.getState().handlers;
}
