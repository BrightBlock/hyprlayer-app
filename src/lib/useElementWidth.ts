import { useCallback, useEffect, useRef, useState } from "react";

/**
 * The live content-box width (px) of one element, tracked with ResizeObserver.
 *
 * The companion to `useViewport`, and the right tool whenever the question is
 * "how much room does this *pane* have" rather than "how wide is the window".
 * The two answers differ by every in-flow column to the pane's left — the nav
 * rail, the project sidebar — which is precisely the space the narrow tier
 * exists to account for, so measuring the window instead of the pane
 * reintroduces the overflow the tier was added to prevent (see lib/layout).
 *
 * Returns a ref callback to attach to the pane and its measured width. The
 * width is seeded from `clientWidth` inside the ref callback — which React runs
 * during commit, when layout is already available — so the first painted frame
 * is measured rather than 0. Attach it to a box with no padding or border, so
 * `clientWidth` and the observed content box agree.
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
