import { EditorView } from "@codemirror/view";

import type { DocHeading } from "./headings";

export const SCROLL_MARGIN = 16;

export function activeHeading(view: EditorView, headings: DocHeading[], scroller: HTMLElement): number {
  if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2) return headings.length - 1;
  const line = scroller.getBoundingClientRect().top + SCROLL_MARGIN + 1;
  let active = -1;
  for (let i = 0; i < headings.length; i++) {
    const pos = Math.min(headings[i].from, view.state.doc.length);
    if (view.documentTop + view.lineBlockAt(pos).top > line) break;
    active = i;
  }
  return active;
}

let jumpToken = 0;

export function jumpToHeading(
  view: EditorView,
  scroller: HTMLElement,
  from: number,
  smooth: boolean,
  onLanded: () => void,
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
    const max = scroller.scrollHeight - scroller.clientHeight;
    const top = Math.min(max, coords.top - scroller.getBoundingClientRect().top + scroller.scrollTop - SCROLL_MARGIN);
    if (Math.abs(top - scroller.scrollTop) < 1) {
      land();
    } else {
      afterScrollEnd(scroller, land);
      scroller.scrollTo({ top, behavior: "smooth" });
    }
  } else {
    view.dispatch({
      selection: { anchor: pos },
      effects: EditorView.scrollIntoView(pos, { y: "start", yMargin: SCROLL_MARGIN }),
    });
    // Queued after CodeMirror's measure frame, which applies the scroll.
    requestAnimationFrame(land);
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
