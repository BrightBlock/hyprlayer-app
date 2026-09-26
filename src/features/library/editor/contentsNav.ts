import { EditorView } from "@codemirror/view";

export const SCROLL_MARGIN = 16;

// Runs every scroll frame: heading tops ascend, so binary search keeps layout reads to log n.
export function activeHeading(count: number, topOf: (i: number) => number, scroller: HTMLElement): number {
  if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2) return count - 1;
  const line = scroller.getBoundingClientRect().top + SCROLL_MARGIN + 1;
  let lo = 0;
  let hi = count - 1;
  let active = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (topOf(mid) <= line) {
      active = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return active;
}

export type ActiveHeading = ReturnType<typeof createActiveHeading>;

// Lets the contents list follow the scrollspy without re-rendering the whole doc view.
export function createActiveHeading() {
  let index = -1;
  const listeners = new Set<() => void>();
  return {
    get: () => index,
    set(next: number) {
      if (next === index) return;
      index = next;
      listeners.forEach((l) => l());
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
  };
}

let jumpToken = 0;

export function jumpToHeading(
  view: EditorView,
  scroller: HTMLElement,
  from: number,
  smooth: boolean,
  onLanded: () => void,
  margin = SCROLL_MARGIN,
) {
  const pos = Math.min(from, view.state.doc.length);
  const token = ++jumpToken;
  // WebKit scrolls to the editor's stale DOM caret on focus despite
  // preventScroll, so focus after landing with the caret parked on the heading.
  const land = () => {
    if (token !== jumpToken) return;
    const { node, offset } = view.domAtPos(pos);
    document.getSelection()?.collapse(node, offset);
    view.focus();
    onLanded();
  };

  // An undrawn heading only has an estimated position (null coords), so let
  // CodeMirror draw and measure it before scrolling there.
  const coords = smooth ? view.coordsAtPos(pos) : null;
  if (coords) {
    view.dispatch({ selection: { anchor: pos } });
    smoothScrollTo(scroller, coords.top, land);
  } else {
    view.dispatch({
      selection: { anchor: pos },
      effects: EditorView.scrollIntoView(pos, { y: "start", yMargin: margin }),
    });
    // Queued after CodeMirror's measure frame, which applies the scroll.
    requestAnimationFrame(land);
  }
}

export function jumpToElement(el: Element, scroller: HTMLElement, smooth: boolean, onLanded: () => void) {
  const token = ++jumpToken;
  const land = () => {
    if (token === jumpToken) onLanded();
  };
  if (smooth) smoothScrollTo(scroller, el.getBoundingClientRect().top, land);
  else {
    scroller.scrollTop += el.getBoundingClientRect().top - scroller.getBoundingClientRect().top - SCROLL_MARGIN;
    land();
  }
}

function smoothScrollTo(scroller: HTMLElement, viewportTop: number, onLanded: () => void) {
  const max = scroller.scrollHeight - scroller.clientHeight;
  const top = Math.min(max, viewportTop - scroller.getBoundingClientRect().top + scroller.scrollTop - SCROLL_MARGIN);
  if (Math.abs(top - scroller.scrollTop) < 1) {
    onLanded();
  } else {
    afterScrollEnd(scroller, onLanded);
    scroller.scrollTo({ top, behavior: "smooth" });
  }
}

function afterScrollEnd(scroller: HTMLElement, fn: () => void) {
  const done = () => {
    clearTimeout(timer);
    scroller.removeEventListener("scrollend", done);
    fn();
  };
  const timer = setTimeout(done, 1000);
  scroller.addEventListener("scrollend", done);
}
