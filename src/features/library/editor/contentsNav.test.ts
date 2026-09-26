import { describe, expect, it } from "vitest";

import { activeHeading, createActiveHeading, SCROLL_MARGIN } from "./contentsNav";

function scroller({ top = 0, scrollTop = 0, clientHeight = 500, scrollHeight = 10_000 } = {}) {
  return {
    scrollTop,
    clientHeight,
    scrollHeight,
    getBoundingClientRect: () => ({ top }),
  } as unknown as HTMLElement;
}

// The scrollspy reads heading tops against this line.
const LINE = SCROLL_MARGIN + 1;

describe("activeHeading", () => {
  // Failure mode 1: a heading below the reading line is reported active.
  it("returns -1 before the first heading reaches the line", () => {
    expect(activeHeading(3, (i) => 100 + i * 100, scroller())).toBe(-1);
  });

  // Failure mode 2: the wrong heading wins when several are above the line.
  it("returns the last heading at or above the line", () => {
    const tops = [-300, -100, LINE, 400, 900];
    expect(activeHeading(tops.length, (i) => tops[i], scroller())).toBe(2);
  });

  // Failure mode 3: the final short section can never become active.
  it("returns the last heading when scrolled to the bottom", () => {
    expect(activeHeading(4, (i) => 100 + i * 100, scroller({ scrollTop: 9_500 }))).toBe(3);
  });

  // Failure mode 4: every scroll frame measures every heading (a layout read and a DOMRect each).
  it("measures a logarithmic number of headings per call", () => {
    let reads = 0;
    const count = 512;
    activeHeading(count, (i) => (reads++, (i - 300) * 50), scroller());
    expect(reads).toBeLessThanOrEqual(10);
  });
});

describe("createActiveHeading", () => {
  // Failure mode 5: subscribers re-render on every scroll frame, not only when the heading changes.
  it("notifies only when the active heading changes", () => {
    const active = createActiveHeading();
    let calls = 0;
    active.subscribe(() => calls++);
    active.set(2);
    active.set(2);
    active.set(3);
    expect(active.get()).toBe(3);
    expect(calls).toBe(2);
  });
});
