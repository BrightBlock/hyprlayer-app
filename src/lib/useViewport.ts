import { useEffect, useState } from "react";

import { isCompact, isNarrow } from "./layout";

/**
 * The live CSS viewport width, and the layout tier it puts the shell in.
 * Read by the route shells to decide whether a side panel renders in flow or
 * as an overlay drawer (see lib/layout). Backed by the DOM resize event: the
 * number that matters is the webview's own viewport, not the OS window frame
 * — on a tiling WM those disagree until the min-size floor is lowered.
 */
export function useViewport(): { width: number; narrow: boolean; compact: boolean } {
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 0 : window.innerWidth,
  );

  useEffect(() => {
    if (typeof window === "undefined") return; // guards a node-environment import
    const sync = () => setWidth(window.innerWidth);
    sync(); // the width may have changed between the lazy initializer and mount
    window.addEventListener("resize", sync);
    return () => window.removeEventListener("resize", sync);
  }, []);

  return { width, narrow: isNarrow(width), compact: isCompact(width) };
}
