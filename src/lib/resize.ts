import type { PointerEvent as ReactPointerEvent } from "react";

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

type EdgeDragOptions = {
  startWidth: number;
  min: number;
  max: number;
  direction: 1 | -1;
  axis?: "x" | "y";
  onResize: (size: number) => void;
  onStart?: () => void;
  onEnd?: () => void;
};

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
