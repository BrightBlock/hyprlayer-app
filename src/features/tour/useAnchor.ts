import { useEffect, useState } from "react";

import { isVisible, type Rect } from "./place";
import type { AnchorId } from "./steps";

/** How long a step waits for its anchor to mount after a reveal before it shows unanchored. */
export const ANCHOR_WAIT_MS = 1500;

export type AnchorState =
  | { status: "searching" }
  | { status: "found"; element: HTMLElement; rect: Rect }
  | { status: "missing" };

export function findAnchor(id: AnchorId, root: ParentNode = document): HTMLElement | null {
  const all = root.querySelectorAll<HTMLElement>("[data-tour]");
  for (const el of Array.from(all)) {
    if (el.dataset.tour !== id) continue;
    const r = el.getBoundingClientRect();
    if (isVisible(r, { width: window.innerWidth, height: window.innerHeight })) return el;
  }
  return null;
}

function rectOf(el: HTMLElement): Rect {
  const r = el.getBoundingClientRect();
  return { left: r.left, top: r.top, width: r.width, height: r.height };
}

const same = (a: Rect, b: Rect) =>
  a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height;

/**
 * Finds the element marked `data-tour={id}` and follows its rect: DOM changes, its own resizes and window
 * resizes re-measure it, with no animated follow. `attempt` restarts the wait (after a reveal or a fix).
 */
export function useAnchor(id: AnchorId | null, attempt = 0): AnchorState {
  const [state, setState] = useState<AnchorState>({ status: "searching" });

  useEffect(() => {
    if (!id) {
      setState({ status: "missing" });
      return;
    }
    setState({ status: "searching" });
    let seenOnce = false;
    let current: HTMLElement | null = null;
    let frame = 0;
    // Coalesced to one measure per frame: a streaming transcript mutates the DOM many times a frame.
    const schedule = () => {
      if (!frame) frame = window.requestAnimationFrame(check);
    };
    const resize = new ResizeObserver(schedule);
    const check = () => {
      frame = 0;
      const el = findAnchor(id);
      if (el !== current) {
        if (current) resize.unobserve(current);
        current = el;
        if (el) resize.observe(el);
      }
      if (el) {
        seenOnce = true;
        const rect = rectOf(el);
        setState((prev) =>
          prev.status === "found" && prev.element === el && same(prev.rect, rect)
            ? prev
            : { status: "found", element: el, rect },
        );
      } else if (seenOnce) {
        setState({ status: "missing" });
      }
    };
    const timer = window.setTimeout(() => {
      if (!current) setState({ status: "missing" });
    }, ANCHOR_WAIT_MS);
    const mutations = new MutationObserver(schedule);
    mutations.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["data-tour", "class", "style", "hidden", "inert"],
    });
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    check();
    return () => {
      window.clearTimeout(timer);
      if (frame) window.cancelAnimationFrame(frame);
      mutations.disconnect();
      resize.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
    };
  }, [id, attempt]);

  return state;
}
