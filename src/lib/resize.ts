import type { PointerEvent as ReactPointerEvent } from "react";

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

type EdgeDragOptions = {
  startWidth: number;
  min: number;
  max: number;
  /** +1 when dragging right grows the panel, -1 when it shrinks it. */
  direction: 1 | -1;
  onResize: (width: number) => void;
};

/**
 * Pointer-drag handler for a panel-resize separator: tracks the pointer,
 * clamps the width, and restores the body cursor/selection on release.
 */
export function startEdgeDrag(
  event: ReactPointerEvent<HTMLElement>,
  { startWidth, min, max, direction, onResize }: EdgeDragOptions,
): void {
  event.preventDefault();
  const startX = event.clientX;
  const previousCursor = document.body.style.cursor;
  const previousSelect = document.body.style.userSelect;

  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";

  const onPointerMove = (moveEvent: PointerEvent) => {
    onResize(clamp(startWidth + direction * (moveEvent.clientX - startX), min, max));
  };
  const onPointerUp = () => {
    document.body.style.cursor = previousCursor;
    document.body.style.userSelect = previousSelect;
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
  };

  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}
