import { useCallback, useEffect, useRef, useState } from "react";

/**
 * The live content-box width (px) of one element, tracked with ResizeObserver.
 *
 * The companion to `useViewport`. Use this when the question is how much room a
 * *pane* has, not how wide the window is. The two differ by every in-flow
 * column to the pane's left (nav rail, project sidebar), which is exactly what
 * the narrow tier accounts for, so measuring the window reintroduces the
 * overflow that tier prevents. See lib/layout.
 *
 * Returns a ref callback for the pane plus its measured width. The width seeds
 * from `clientWidth` inside the ref callback, which React runs during commit
 * with layout available, so the first painted frame is measured rather than 0.
 * Attach it to a box with no padding or border so the two agree.
 */
export function useElementWidth<T extends HTMLElement>(): {
  ref: (node: T | null) => void;
  width: number;
} {
  const [width, setWidth] = useState(0);
  const observer = useRef<ResizeObserver | null>(null);

  const ref = useCallback((node: T | null) => {
    observer.current?.disconnect();
    observer.current = null;
    // No node (unmount / detach), or a node environment with no ResizeObserver.
    if (!node || typeof ResizeObserver === "undefined") return;
    setWidth(node.clientWidth);
    const next = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (entry) setWidth(entry.contentRect.width);
    });
    next.observe(node);
    observer.current = next;
  }, []);

  // Ref callbacks already disconnect on detach; this covers an unmount where
  // React tears the tree down without calling the callback back with null.
  useEffect(() => () => observer.current?.disconnect(), []);

  return { ref, width };
}
