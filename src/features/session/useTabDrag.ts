/**
 * Pointer-drag for agent tabs. Pointer-based rather than HTML5
 * drag-and-drop — WebKit/Tauri intercept native drags for file drops, so
 * they're unreliable for in-app reordering.
 *
 * Two behaviors, both off the same gesture:
 *   • reorder within a pane — while the cursor stays over the tab's own pane,
 *     the tab live-reorders as it passes neighbours' centers.
 *   • move across the split — dropping over the OTHER pane reassigns the tab.
 *
 * The pane under the pointer is found by hit-testing for a `[data-pane]`
 * ancestor (both the tab strips and the content panes carry it), and order is
 * read from the `[data-tab-id]` pills in that pane's strip.
 */

import { useEffect, useRef, useState, type RefObject, type PointerEvent as ReactPointerEvent } from "react";

export type Pane = "main" | "right";

/** Per-pane drag handlers, passed via a ref so the long-lived pointer
 *  listeners always reach the latest closures without re-binding. */
export type TabDragHandlers = {
  reorder: (id: number, pane: Pane, toIndex: number) => void;
  move: (id: number, toPane: Pane) => void;
};

/** Which split pane sits under a screen point, or null when over neither. */
export function paneAtPoint(x: number, y: number): Pane | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  const pane = el?.closest<HTMLElement>("[data-pane]")?.dataset.pane;
  return pane === "main" || pane === "right" ? pane : null;
}

/** Insertion index for the dragged tab within `pane`'s strip: the count of
 *  OTHER pills whose horizontal center sits left of the pointer. */
function dropIndexAt(pane: Pane, x: number, draggedId: number): number {
  const pills = Array.from(
    document.querySelectorAll<HTMLElement>(`[data-pane="${pane}"] [data-tab-id]`),
  );
  let index = 0;
  for (const pill of pills) {
    if (Number(pill.dataset.tabId) === draggedId) continue;
    const rect = pill.getBoundingClientRect();
    if (x > rect.left + rect.width / 2) index++;
  }
  return index;
}

export type TabDragState = { id: number; title: string; x: number; y: number } | null;

export function useTabDrag(handlersRef: RefObject<TabDragHandlers>) {
  // The live drag (for the floating ghost) and which OTHER pane the cursor is
  // over (for the cross-pane drop-target highlight).
  const [drag, setDrag] = useState<TabDragState>(null);
  const [dropPane, setDropPane] = useState<Pane | null>(null);
  // Set briefly on drop so the click that ends a drag doesn't also select the
  // tab (the pill's onClick checks this).
  const suppressClickRef = useRef(false);
  const cleanupRef = useRef<() => void>(() => {});

  // Tear down stray window listeners if we unmount mid-drag.
  useEffect(() => () => cleanupRef.current(), []);

  const startDrag = (
    e: ReactPointerEvent,
    info: { id: number; title: string; fromPane: Pane },
  ) => {
    // Left button only; never start from the close button (it has its own job).
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input")) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let started = false;
    // Last index we reordered to, so we only write the store on a real change.
    let lastIndex = -1;

    const onMove = (ev: PointerEvent) => {
      if (!started) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        started = true;
      }
      const over = paneAtPoint(ev.clientX, ev.clientY);
      if (over === info.fromPane) {
        // Same pane → live reorder; no cross-pane highlight.
        setDropPane(null);
        const target = dropIndexAt(info.fromPane, ev.clientX, info.id);
        if (target !== lastIndex) {
          lastIndex = target;
          handlersRef.current?.reorder(info.id, info.fromPane, target);
        }
      } else {
        setDropPane(over && over !== info.fromPane ? over : null);
      }
      setDrag({ id: info.id, title: info.title, x: ev.clientX, y: ev.clientY });
    };
    const onUp = (ev: PointerEvent) => {
      cleanup();
      if (started) {
        suppressClickRef.current = true;
        setTimeout(() => {
          suppressClickRef.current = false;
        }, 0);
        const over = paneAtPoint(ev.clientX, ev.clientY);
        // Same-pane drops already reordered live; only a cross-pane drop is
        // left to apply here.
        if (over && over !== info.fromPane) handlersRef.current?.move(info.id, over);
      }
      setDrag(null);
      setDropPane(null);
    };
    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      cleanupRef.current = () => {};
    };

    cleanupRef.current = cleanup;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  return { drag, dropPane, suppressClickRef, startDrag };
}
