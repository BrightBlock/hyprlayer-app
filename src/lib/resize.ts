import type { PointerEvent as ReactPointerEvent } from "react";

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

type EdgeDragOptions = {
  /** Starting size (px) along the drag axis. */
  startWidth: number;
  min: number;
  max: number;
  /** +1 when dragging toward the axis-positive direction (right / down) grows
   *  the panel, -1 when it shrinks it. */
  direction: 1 | -1;
  /** Drag axis — horizontal (width, default) or vertical (height). */
  axis?: "x" | "y";
  onResize: (size: number) => void;
  /** Fired on pointer-down / release — e.g. to suppress a transition while
   *  dragging so the panel tracks the cursor instead of trailing it. */
  onStart?: () => void;
  onEnd?: () => void;
};

/**
 * Pointer-drag handler for a panel-resize separator: tracks the pointer,
 * clamps the size, and restores the body cursor/selection on release. Works on
 * either axis (horizontal width or vertical height).
 */
export function startEdgeDrag(
  event: ReactPointerEvent<HTMLElement>,
  { startWidth, min, max, direction, axis = "x", onResize, onStart, onEnd }: EdgeDragOptions,
): void {
  event.preventDefault();
  const horizontal = axis === "x";
  const startPos = horizontal ? event.clientX : event.clientY;
  const previousCursor = document.body.style.cursor;
  const previousSelect = document.body.style.userSelect;

  document.body.style.cursor = horizontal ? "col-resize" : "row-resize";
  document.body.style.userSelect = "none";
  onStart?.();

  const onPointerMove = (moveEvent: PointerEvent) => {
    const pos = horizontal ? moveEvent.clientX : moveEvent.clientY;
    onResize(clamp(startWidth + direction * (pos - startPos), min, max));
  };
  const onPointerUp = () => {
    document.body.style.cursor = previousCursor;
    document.body.style.userSelect = previousSelect;
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
    onEnd?.();
  };

  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}
