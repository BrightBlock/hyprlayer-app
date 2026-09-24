import { useEffect, useState } from "react";

const QUERY = "(prefers-reduced-motion: reduce)";

function query(): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(QUERY)
    : null;
}

export function useReducedMotion(): boolean {
  const [reduce, setReduce] = useState(() => query()?.matches ?? false);
  useEffect(() => {
    const mq = query();
    if (!mq) return;
    const onChange = () => setReduce(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return reduce;
}
