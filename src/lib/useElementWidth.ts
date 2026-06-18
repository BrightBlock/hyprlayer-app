import { useEffect, useState, type RefObject } from "react";

/**
 * Track an element's content-box width via ResizeObserver. Used to collapse
 * toolbars to icon-only when their *container* (a pane/header) gets narrow —
 * which a viewport media query can't capture (a split halves a pane while the
 * window stays wide), and which Tailwind container queries can't drive here
 * without `container-type` establishing a containing block that would reparent
 * the existing `position: fixed` dropdown backdrops.
 *
 * Returns 0 until first measured, so guard thresholds with `w > 0`.
 */
export function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) setWidth(entry.contentRect.width);
    });
    observer.observe(el);
    setWidth(el.getBoundingClientRect().width);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}
