import { useEffect, useState } from "react";

import { isCompact, isNarrow } from "./layout";

export function useViewport(): { width: number; narrow: boolean; compact: boolean } {
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 0 : window.innerWidth,
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    const sync = () => setWidth(window.innerWidth);
    sync();
    window.addEventListener("resize", sync);
    return () => window.removeEventListener("resize", sync);
  }, []);

  return { width, narrow: isNarrow(width), compact: isCompact(width) };
}
