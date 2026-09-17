
import { useEffect, useRef, useState, type RefObject, type PointerEvent as ReactPointerEvent } from "react";

import { edgeZone, type Edge, type PaneId } from "./paneTree";

export type TabDragHandlers = {
  reorder: (id: number, pane: PaneId, toIndex: number) => void;
  move: (id: number, toPane: PaneId) => void;
  split: (id: number, paneId: PaneId, edge: Edge) => void;
  canEdgeSplit: (id: number, paneId: PaneId, edge: Edge) => boolean;
};

export type DropTarget = { paneId: PaneId; edge: Edge | null } | null;

export function paneAtPoint(x: number, y: number): PaneId | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  return el?.closest<HTMLElement>("[data-pane]")?.dataset.pane || null;
}

function contentRectAt(pane: PaneId): DOMRect | null {
  return (
    document
      .querySelector<HTMLElement>(`[data-pane="${pane}"] [data-pane-content]`)
      ?.getBoundingClientRect() ?? null
  );
}

function dropIndexAt(pane: PaneId, x: number, draggedId: number): number {
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
  const [drag, setDrag] = useState<TabDragState>(null);
  const [dropTarget, setDropTarget] = useState<DropTarget>(null);
  const suppressClickRef = useRef(false);
  const cleanupRef = useRef<() => void>(() => {});

  useEffect(() => () => cleanupRef.current(), []);

  const startDrag = (
    e: ReactPointerEvent,
    info: { id: number; title: string; fromPane: PaneId },
  ) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input")) return;
    cleanupRef.current();
    const startX = e.clientX;
    const startY = e.clientY;
    let started = false;
    let lastIndex = -1;

    const paneAndEdgeAt = (x: number, y: number): { over: PaneId | null; edge: Edge | null } => {
      const over = paneAtPoint(x, y);
      if (!over) return { over: null, edge: null };
      const rect = contentRectAt(over);
      const raw = rect ? edgeZone(rect, x, y) : null;
      const edge = raw && handlersRef.current?.canEdgeSplit(info.id, over, raw) ? raw : null;
      return { over, edge };
    };

    const onMove = (ev: PointerEvent) => {
      if (!started) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        started = true;
      }
      const { over, edge } = paneAndEdgeAt(ev.clientX, ev.clientY);
      if (over && edge) {
        setDropTarget({ paneId: over, edge });
      } else if (over === info.fromPane) {
        setDropTarget(null);
        const target = dropIndexAt(info.fromPane, ev.clientX, info.id);
        if (target !== lastIndex) {
          lastIndex = target;
          handlersRef.current?.reorder(info.id, info.fromPane, target);
        }
      } else {
        setDropTarget(over && over !== info.fromPane ? { paneId: over, edge: null } : null);
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
        const { over, edge } = paneAndEdgeAt(ev.clientX, ev.clientY);
        if (over && edge) {
          handlersRef.current?.split(info.id, over, edge);
        } else if (over && over !== info.fromPane) {
          handlersRef.current?.move(info.id, over);
        }
      }
      setDrag(null);
      setDropTarget(null);
    };
    const onCancel = () => {
      cleanup();
      setDrag(null);
      setDropTarget(null);
    };
    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      cleanupRef.current = () => {};
    };

    cleanupRef.current = cleanup;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  };

  return { drag, dropTarget, suppressClickRef, startDrag };
}
