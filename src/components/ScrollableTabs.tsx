import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { ChevronLeftIcon, ChevronRightIcon } from "./icons";

/**
 * A horizontally-scrollable tab strip that reveals a chevron at either edge once
 * its tabs overflow that side — the affordance for a strip that has grown past
 * the bar's width. Each chevron both signals "more tabs this way" and nudges the
 * scroll when clicked.
 *
 * The chevrons overlay the scroll area's edges (absolute + a fade to the bar
 * colour) so they never reflow the tabs when toggling on/off, and the strip
 * keeps its hidden native scrollbar so trackpad / shift-wheel scrolling still
 * works underneath. `trailing` (e.g. the new-tab button) is pinned OUTSIDE the
 * scroll area — past the right chevron — so it stays put and reachable however
 * far the tabs scroll.
 *
 * Layout note: the wrapper takes the flex role the bare scroller used to have
 * (`min-w-0` so it can shrink and let the inner scroller overflow). `grow` adds
 * `flex-1` for call sites — like the split's right pane — that must fill their
 * strip and push a trailing control to the far edge.
 */
export function ScrollableTabStrip({
  children,
  trailing,
  activeId,
  grow = false,
}: {
  children: ReactNode;
  /** Always-visible content pinned to the right of the scroll area, past the
   *  right chevron — so a control like "new tab" never scrolls out of reach. */
  trailing?: ReactNode;
  /** The selected tab's id; when it changes, that tab is scrolled into view so a
   *  freshly-opened (or newly-focused) tab is never left off-screen. Pills must
   *  carry a matching `data-tab-id`. */
  activeId?: number | null;
  /** Fill the parent flex row (right-pane strip); omit for content-width (main). */
  grow?: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [canLeft, setCanLeft] = useState(false);
  const [canRight, setCanRight] = useState(false);

  const sync = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    // 1px slack absorbs sub-pixel rounding so a chevron never sticks on at a
    // hard scroll boundary.
    setCanLeft(el.scrollLeft > 1);
    setCanRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  }, []);

  useLayoutEffect(() => {
    const viewport = scrollRef.current;
    const content = contentRef.current;
    if (!viewport || !content) return;
    sync();
    // Observe both boxes: the content grows/shrinks as tabs are added, renamed,
    // or closed; the viewport changes when the window or a split pane resizes.
    const ro = new ResizeObserver(sync);
    ro.observe(viewport);
    ro.observe(content);
    return () => ro.disconnect();
  }, [sync]);

  // Reveal the active tab whenever it changes — covers opening a tab (it lands
  // at the far end) and focusing one that's scrolled off either edge.
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
