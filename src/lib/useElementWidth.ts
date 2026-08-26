import { useCallback, useEffect, useRef, useState } from "react";

export function useElementWidth<T extends HTMLElement>(): {
  ref: (node: T | null) => void;
  width: number;
} {
  const [width, setWidth] = useState(0);
  const observer = useRef<ResizeObserver | null>(null);

  const ref = useCallback((node: T | null) => {
    observer.current?.disconnect();
    observer.current = null;
    if (!node || typeof ResizeObserver === "undefined") return;
    setWidth(node.clientWidth);
    const next = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (entry) setWidth(entry.contentRect.width);
    });
    next.observe(node);
    observer.current = next;
  }, []);

  useEffect(() => () => observer.current?.disconnect(), []);

  return { ref, width };
}
