/**
 * Pointer-drag to reorder the project sidebar. Pointer-based rather than HTML5
 * drag-and-drop (WebKit/Tauri intercept native drags for file drops), and
 * transform-based so the reorder reads smoothly: the grabbed block, a project
 * header together with its nested workstreams, lifts and tracks the cursor,
 * while the other blocks slide via CSS transitions to open the gap where it will
 * land. Nothing changes in the DOM order until the drop is committed.
 */

import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";

/** Passed via a ref so the long-lived pointer listeners always reach the latest
 *  closure (current project list + mutation) without re-binding. */
export type ProjectDragHandlers = {
  /** Persist the move once the drag ends. `from`/`to` are indices in the
   *  committed order; a no-op is ignored by the handler. */
  commit: (from: number, to: number) => void;
};

/** Live drag geometry, consumed by the sidebar to position every block. */
export type ProjectDragState = {
  path: string;
  /** translateY (px) for the dragged block, so it follows the cursor. */
  dy: number;
  /** Committed index of the dragged block. */
  srcIndex: number;
  /** Insertion index it will drop to (0-based among all blocks). */
  targetIndex: number;
  /** Slot height (px) the other blocks shift by to open the gap. */
  height: number;
} | null;

type Snapshot = { path: string; top: number; height: number };

/** The sidebar blocks' untransformed layout, captured once at drag start. */
function snapshotBlocks(): Snapshot[] {
  return Array.from(document.querySelectorAll<HTMLElement>("[data-project-path]")).map((el) => {
    const rect = el.getBoundingClientRect();
    return { path: el.dataset.projectPath ?? "", top: rect.top, height: rect.height };
  });
}

/** Insertion index for the dragged block: the count of OTHER blocks whose center
 *  sits above the pointer. Read from the drag-start snapshot (not live rects) so
 *  the visual transforms never feed back into the hit-test. */
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
  // Set briefly on drop so the click that ends a drag doesn't also navigate
  // (the project header is a <Link>; its onClick checks this).
  const suppressClickRef = useRef(false);
  const cleanupRef = useRef<() => void>(() => {});

  // Tear down stray window listeners if we unmount mid-drag.
  useEffect(() => () => cleanupRef.current(), []);

  const startDrag = (e: ReactPointerEvent, info: { path: string }) => {
    // Left button only; never start from an inner control (the "+" new-workstream
    // link, the copy-files warning), those carry `data-no-drag`.
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
        // 5px dead-zone so a plain click still navigates.
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        started = true;
        // Snapshot the layout once, before any block has shifted.
        snapshot = snapshotBlocks();
        srcIndex = snapshot.findIndex((b) => b.path === info.path);
        const srcTop = snapshot[srcIndex]?.top ?? 0;
        const nextTop = snapshot[srcIndex + 1]?.top;
        // Slot pitch of the dragged block (its height plus the row gap), so the
        // gap the others open matches the space it vacates. Last row: its height.
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
