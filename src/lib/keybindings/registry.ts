/**
 * Command registry: components register a handler for a command id while they're
 * mounted; the global dispatcher (`useKeybindings`) looks the id up and runs it.
 *
 * Whether a handler is currently registered IS the runtime context predicate —
 * e.g. *Stop turn* is only registered while a session is running, so the same
 * chord is simply inert elsewhere. A null handler means "not available right
 * now", so the dispatcher lets the keystroke pass through.
 *
 * Each id holds a *stack* of handlers, not a single slot: if two components
 * register the same command at once (split panes, a modal over a page), the
 * latest mount shadows the earlier one and unmounting it restores the shadowed
 * handler — instead of one overwriting the other and the unmount deleting the
 * binding entirely.
 */

import { useEffect, useRef } from "react";
import { create } from "zustand";

type RegistryState = { handlers: Record<string, Array<() => void>> };

const useRegistry = create<RegistryState>(() => ({ handlers: {} }));

/**
 * Push `handler` onto the stack for `id`; returns a disposer that removes
 * exactly this entry (each call closes over a fresh closure, so identity is
 * unique even when the same function is registered twice).
 */
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
    return register(id, () => ref.current?.());
  }, [id, enabled]);
}

/** Virtual command id for Escape — not user-rebindable, but it rides the same
 *  handler stack so a dialog/menu over a page shadows the page's own Escape. */
export const ESCAPE_COMMAND_ID = "ui.escape";

/** Register an Escape handler while mounted (`null` = inert here). */
export function useEscape(handler: (() => void) | null): void {
  useCommand(ESCAPE_COMMAND_ID, handler);
}

/** Run the topmost handler registered for `id`. Returns true iff one was registered. */
export function runCommand(id: string): boolean {
  const stack = useRegistry.getState().handlers[id];
  const handler = stack?.[stack.length - 1];
  if (!handler) return false;
  handler();
  return true;
}

/** Test-only: whether a handler is currently registered for `id`. */
export function isRegistered(id: string): boolean {
  return (useRegistry.getState().handlers[id]?.length ?? 0) > 0;
}
