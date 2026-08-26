
import { useEffect, useRef, useState, type RefObject, type PointerEvent as ReactPointerEvent } from "react";

export type Pane = "main" | "right";

export type TabDragHandlers = {
  reorder: (id: number, pane: Pane, toIndex: number) => void;
  move: (id: number, toPane: Pane) => void;
};

export function paneAtPoint(x: number, y: number): Pane | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  const pane = el?.closest<HTMLElement>("[data-pane]")?.dataset.pane;
  return pane === "main" || pane === "right" ? pane : null;
}

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
  const [drag, setDrag] = useState<TabDragState>(null);
  const [dropPane, setDropPane] = useState<Pane | null>(null);
  const suppressClickRef = useRef(false);
  const cleanupRef = useRef<() => void>(() => {});

  useEffect(() => () => cleanupRef.current(), []);

  const startDrag = (
    e: ReactPointerEvent,
    info: { id: number; title: string; fromPane: Pane },
  ) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input")) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let started = false;
    let lastIndex = -1;

    const onMove = (ev: PointerEvent) => {
      if (!started) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        started = true;
      }
      const over = paneAtPoint(ev.clientX, ev.clientY);
      if (over === info.fromPane) {
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
