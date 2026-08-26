
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";

export type ProjectDragHandlers = {
  commit: (from: number, to: number) => void;
};

export type ProjectDragState = {
  path: string;
  dy: number;
  srcIndex: number;
  targetIndex: number;
  height: number;
} | null;

type Snapshot = { path: string; top: number; height: number };

function snapshotBlocks(): Snapshot[] {
  return Array.from(document.querySelectorAll<HTMLElement>("[data-project-path]")).map((el) => {
    const rect = el.getBoundingClientRect();
    return { path: el.dataset.projectPath ?? "", top: rect.top, height: rect.height };
  });
}

function targetIndexAt(y: number, snapshot: Snapshot[], draggedPath: string): number {
  let index = 0;
  for (const b of snapshot) {
    if (b.path === draggedPath) continue;
    if (y > b.top + b.height / 2) index++;
  }
  return index;
}

export function useProjectDrag(handlersRef: RefObject<ProjectDragHandlers>) {
  const [drag, setDrag] = useState<ProjectDragState>(null);
  const suppressClickRef = useRef(false);
  const cleanupRef = useRef<() => void>(() => {});

  useEffect(() => () => cleanupRef.current(), []);

  const startDrag = (e: ReactPointerEvent, info: { path: string }) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("[data-no-drag]")) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let started = false;
    let snapshot: Snapshot[] = [];
    let srcIndex = 0;
    let height = 0;
    let lastTarget = -1;

    const onMove = (ev: PointerEvent) => {
      if (!started) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        started = true;
        snapshot = snapshotBlocks();
        srcIndex = snapshot.findIndex((b) => b.path === info.path);
        const srcTop = snapshot[srcIndex]?.top ?? 0;
        const nextTop = snapshot[srcIndex + 1]?.top;
        height = nextTop != null ? nextTop - srcTop : (snapshot[srcIndex]?.height ?? 0);
      }
      const targetIndex = targetIndexAt(ev.clientY, snapshot, info.path);
      lastTarget = targetIndex;
      setDrag({ path: info.path, dy: ev.clientY - startY, srcIndex, targetIndex, height });
    };
    const onUp = () => {
      cleanup();
      if (started) {
        suppressClickRef.current = true;
        setTimeout(() => {
          suppressClickRef.current = false;
        }, 0);
        if (lastTarget >= 0) handlersRef.current?.commit(srcIndex, lastTarget);
      }
      setDrag(null);
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

  return { drag, suppressClickRef, startDrag };
}
