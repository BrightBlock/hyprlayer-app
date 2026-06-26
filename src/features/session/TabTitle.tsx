/**
 * The tab pill's title text, with a one-shot typewriter reveal when the title
 * was just auto-generated (see [`generateTabTitle`]). Every other title change —
 * manual rename, switching tabs, restore from the log, reduced-motion users —
 * renders instantly. The "animate" signal is claimed once via
 * [`consumeTabTitleAnimation`], so it never replays on remount.
 */

import { useEffect, useRef, useState } from "react";

import { consumeTabTitleAnimation } from "./tabTitles";

/** Per-character reveal cadence (ms). ~28ms reads as a brisk type-out without
 *  feeling slow on a long title. */
const TYPE_MS = 28;

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

export function TabTitle({
  workstreamId,
  shellId,
  title,
}: {
  workstreamId: string;
  shellId: number;
  title: string;
}) {
  const [display, setDisplay] = useState(title);
  const [typing, setTyping] = useState(false);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (timer.current) clearInterval(timer.current);
    // Claim the one-shot auto-title signal inside a rAF, not in the effect body.
    // React StrictMode double-invokes effects (setup→cleanup→setup); a bare
    // consume here would be claimed-then-cancelled on the first pass and the
    // typewriter would never play. Cancelling the frame in cleanup means only the
    // surviving pass consumes — exactly once. (Mirrors SessionChat's composer-
    // focus effect.)
    const raf = requestAnimationFrame(() => {
      // Consume first (always clears the signal), then decide whether to animate.
      const animate =
        consumeTabTitleAnimation(workstreamId, shellId) && !prefersReducedMotion();
      if (!animate) {
        // Ordinary title change (manual rename, restore, tab switch) or reduced
        // motion: snap straight to the final title.
        setDisplay(title);
        setTyping(false);
        return;
      }
      setTyping(true);
      setDisplay("");
      let i = 0;
      timer.current = setInterval(() => {
        i += 1;
        setDisplay(title.slice(0, i));
        if (i >= title.length) {
          if (timer.current) clearInterval(timer.current);
          timer.current = null;
          setTyping(false);
        }
      }, TYPE_MS);
    });
    return () => {
      cancelAnimationFrame(raf);
      if (timer.current) clearInterval(timer.current);
      timer.current = null;
    };
  }, [title, workstreamId, shellId]);

  return (
    <>
      {display}
      {typing && (
        <span
          aria-hidden
          className="ml-px inline-block w-px animate-pulse self-center bg-current align-middle"
          style={{ height: "0.85em" }}
        />
      )}
    </>
  );
}
