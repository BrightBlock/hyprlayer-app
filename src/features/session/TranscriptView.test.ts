// @vitest-environment jsdom
import { describe, expect, it } from "vitest";

import { stickAfterScroll } from "./TranscriptView";

// stickAfterScroll(prevTop, top, gap, wasStuck) — `gap` is the distance to the
// bottom (scrollHeight - clientHeight - scrollTop). Releases only on a genuine
// scroll-up; a scrollTop drop that keeps the gap ~0 is a layout clamp, not a
// gesture, and must hold the stick.
describe("stickAfterScroll", () => {
  it("releases on a genuine scroll-up (scrollTop dropped, real gap opened)", () => {
    expect(stickAfterScroll(1000, 500, 500, true)).toBe(false);
  });

  it("holds the stick when the viewport grows (composer shrank) and scrollTop clamps", () => {
    // Was pinned at the bottom; the composer collapsed, so clientHeight grew and
    // the browser clamped scrollTop down — but the gap stays ~0. The reported bug:
    // this used to read as a scroll-up and unpin the view.
    expect(stickAfterScroll(1000, 960, 0, true)).toBe(true);
  });

  it("holds the stick when content shrinks and scrollTop clamps (gap ~0)", () => {
    expect(stickAfterScroll(1000, 999, 1, true)).toBe(true);
  });

  it("does not release on streaming growth (gap opens but scrollTop did not drop)", () => {
    // Content grew below the fold before the pin loop caught up: large gap, but
    // scrollTop held — not a gesture, so the stick stays.
    expect(stickAfterScroll(1000, 1000, 200, true)).toBe(true);
  });

  it("re-sticks once the user scrolls back within RESTICK_PX of the bottom", () => {
    expect(stickAfterScroll(100, 1000, 10, false)).toBe(true);
  });

  it("treats a sub-2px scrollTop drop at the bottom as a clamp, not a scroll-up", () => {
    expect(stickAfterScroll(1000, 998, 2, true)).toBe(true);
  });

  it("stays released far from the bottom when nothing moved (e.g. a resize event)", () => {
    expect(stickAfterScroll(200, 200, 500, false)).toBe(false);
  });

  it("keeps releasing as the user scrolls further up while already unstuck", () => {
    expect(stickAfterScroll(500, 300, 700, false)).toBe(false);
  });
});
