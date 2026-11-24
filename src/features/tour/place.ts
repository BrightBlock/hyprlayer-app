import type { Side } from "./steps";

export type Rect = { left: number; top: number; width: number; height: number };
type Size = { width: number; height: number };

export type Placement = { left: number; top: number; side: Side; caret: number };

const GAP = 10;
const MARGIN = 8;
const CARET = 9;
const CARET_INSET = 12;

const OPPOSITE: Record<Side, Side> = { top: "bottom", bottom: "top", left: "right", right: "left" };

function room(a: Rect, side: Side, view: Size): number {
  switch (side) {
    case "top":
      return a.top - GAP - MARGIN;
    case "bottom":
      return view.height - (a.top + a.height) - GAP - MARGIN;
    case "left":
      return a.left - GAP - MARGIN;
    case "right":
      return view.width - (a.left + a.width) - GAP - MARGIN;
  }
}

const need = (side: Side, panel: Size) => (side === "top" || side === "bottom" ? panel.height : panel.width);

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)));

/**
 * Places the coach mark beside its anchor on the preferred side, flipping when that side lacks room, and
 * keeps it inside the viewport. `caret` is the caret's offset along the edge that faces the anchor.
 */
export function placeCoach(anchor: Rect, panel: Size, preferred: Side, view: Size): Placement {
  const sides: Side[] = [preferred, OPPOSITE[preferred], ...(["top", "bottom", "left", "right"] as Side[])];
  const side = sides.find((s) => room(anchor, s, view) >= need(s, panel)) ?? preferred;
  const cx = anchor.left + anchor.width / 2;
  const cy = anchor.top + anchor.height / 2;
  let left: number;
  let top: number;
  if (side === "top" || side === "bottom") {
    left = clamp(cx - panel.width / 2, MARGIN, view.width - panel.width - MARGIN);
    top = side === "top" ? anchor.top - GAP - panel.height : anchor.top + anchor.height + GAP;
    top = clamp(top, MARGIN, view.height - panel.height - MARGIN);
    const caret = clamp(cx - left - CARET / 2, CARET_INSET, panel.width - CARET_INSET - CARET);
    return { left, top, side, caret };
  }
  top = clamp(cy - panel.height / 2, MARGIN, view.height - panel.height - MARGIN);
  left = side === "left" ? anchor.left - GAP - panel.width : anchor.left + anchor.width + GAP;
  left = clamp(left, MARGIN, view.width - panel.width - MARGIN);
  const caret = clamp(cy - top - CARET / 2, CARET_INSET, panel.height - CARET_INSET - CARET);
  return { left, top, side, caret };
}

/** An anchor that is off screen or collapsed to nothing can't be pointed at. */
export function isVisible(rect: Rect, view: Size): boolean {
  if (rect.width <= 0 || rect.height <= 0) return false;
  return rect.left < view.width && rect.top < view.height && rect.left + rect.width > 0 && rect.top + rect.height > 0;
}
