import type { EditorView } from "@codemirror/view";

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

function targetTop(view: EditorView, scroller: HTMLElement, pos: number): number {
  return view.documentTop + view.lineBlockAt(pos).top - scroller.getBoundingClientRect().top + scroller.scrollTop - SCROLL_MARGIN;
}

export function jumpToHeading(view: EditorView, scroller: HTMLElement, from: number, smooth: boolean) {
  const pos = Math.min(from, view.state.doc.length);

  // CodeMirror only has a real measurement for lines it has drawn; a heading
  // outside the current viewport gets an estimated position (our headings
  // vary a lot in height, so the estimate is often well off) until that area
  // is actually rendered. An instant jump there forces the measurement, then
  // a second pass corrects the position with the requested scroll behavior —
  // otherwise the first click lands short/long and only a second click (now
  // starting from a measured position) is accurate.
  scroller.scrollTo({ top: targetTop(view, scroller, pos), behavior: "auto" });
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      scroller.scrollTo({ top: targetTop(view, scroller, pos), behavior: smooth ? "smooth" : "auto" });
    });
  });

  view.dispatch({ selection: { anchor: pos } });
  view.focus();
}
