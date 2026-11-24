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

// The largest size that still leaves `keep` px of the container for its neighbour.
// An unmeasured container (0) leaves the cap alone.
export function maxBeside(container: number, keep: number, min: number, max: number): number {
  if (container <= 0) return max;
  return clamp(container - keep, min, max);
}

const KEY_STEP = 16;
const KEY_STEP_LARGE = 64;

// The size an arrow, Home or End key moves a separator to, or null for any other key.
// Arrows move the handle; `direction` maps that onto the panel's size.
export function keyedSize(
  key: string,
  shift: boolean,
  { size, min, max, direction, axis }: { size: number; min: number; max: number; direction: 1 | -1; axis: "x" | "y" },
): number | null {
  const step = shift ? KEY_STEP_LARGE : KEY_STEP;
  const back = axis === "x" ? "ArrowLeft" : "ArrowUp";
  const forward = axis === "x" ? "ArrowRight" : "ArrowDown";
  if (key === back) return clamp(size - direction * step, min, max);
  if (key === forward) return clamp(size + direction * step, min, max);
  if (key === "Home") return min;
  if (key === "End") return max;
  return null;
}
