
import { useEffect, useRef, useState } from "react";

import { consumeTabTitleAnimation } from "./tabTitles";

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
    const raf = requestAnimationFrame(() => {
      const animate =
        consumeTabTitleAnimation(workstreamId, shellId) && !prefersReducedMotion();
      if (!animate) {
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
