import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { ChevronLeftIcon, ChevronRightIcon } from "./icons";

export function ScrollableTabStrip({
  children,
  trailing,
  activeId,
  grow = false,
}: {
  children: ReactNode;
  trailing?: ReactNode;
  activeId?: number | null;
  grow?: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [canLeft, setCanLeft] = useState(false);
  const [canRight, setCanRight] = useState(false);

  const sync = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    setCanLeft(el.scrollLeft > 1);
    setCanRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  }, []);

  useLayoutEffect(() => {
    const viewport = scrollRef.current;
    const content = contentRef.current;
    if (!viewport || !content) return;
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(viewport);
    ro.observe(content);
    return () => ro.disconnect();
  }, [sync]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || activeId == null) return;
    const pill = el.querySelector<HTMLElement>(`[data-tab-id="${activeId}"]`);
    pill?.scrollIntoView({ inline: "nearest", block: "nearest", behavior: "smooth" });
  }, [activeId]);

  const nudge = (direction: -1 | 1) => {
    const el = scrollRef.current;
    if (el) el.scrollBy({ left: direction * el.clientWidth * 0.7, behavior: "smooth" });
  };

  return (
    <div className={`flex min-w-0 items-center gap-1 ${grow ? "flex-1" : ""}`}>
      <div className="relative flex min-w-0 items-center">
        {canLeft && (
          <button
            type="button"
            onClick={() => nudge(-1)}
            aria-label="Scroll tabs left"
            className="absolute inset-y-0 left-0 z-10 flex items-center bg-gradient-to-r from-panel via-panel to-transparent pl-1 pr-6 text-ink-dim hover:text-ink"
          >
            <ChevronLeftIcon />
          </button>
        )}
        <div
          ref={scrollRef}
          onScroll={sync}
          className="no-scrollbar flex min-w-0 items-center overflow-x-auto"
        >
          <div ref={contentRef} className="flex items-center gap-1">
            {children}
          </div>
        </div>
        {canRight && (
          <button
            type="button"
            onClick={() => nudge(1)}
            aria-label="Scroll tabs right"
            className="absolute inset-y-0 right-0 z-10 flex items-center bg-gradient-to-l from-panel via-panel to-transparent pl-6 pr-1 text-ink-dim hover:text-ink"
          >
            <ChevronRightIcon />
          </button>
        )}
      </div>
      {trailing}
    </div>
  );
}
