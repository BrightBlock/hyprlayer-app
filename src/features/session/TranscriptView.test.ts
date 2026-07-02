// @vitest-environment jsdom
import { describe, expect, it } from "vitest";

import { insideNestedScroller, releaseOnWheel, stickAfterScroll } from "./TranscriptView";

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

// releaseOnWheel(deltaX, deltaY, ctrlKey, scrollTop) — should this wheel event
// drop the bottom-stick? Only a genuine upward, vertical-dominant wheel with
// scrollback above it can.
describe("releaseOnWheel", () => {
  it("releases on a plain wheel/trackpad scroll-up", () => {
    expect(releaseOnWheel(0, -40, false, 500)).toBe(true);
  });

  it("ignores scroll-down", () => {
    expect(releaseOnWheel(0, 40, false, 500)).toBe(false);
  });

  it("ignores pinch-zoom (ctrl+wheel)", () => {
    expect(releaseOnWheel(0, -40, true, 500)).toBe(false);
  });

  it("ignores a horizontal pan that leaks a little negative deltaY", () => {
    expect(releaseOnWheel(-60, -3, false, 500)).toBe(false);
  });

  it("ignores a wheel-up with no scrollback above (short transcript)", () => {
    // The old handler unpinned here, and with the content not overflowing no
    // scroll event ever fires again — the pin was dead for the whole session.
    expect(releaseOnWheel(0, -40, false, 0)).toBe(false);
  });
});

// insideNestedScroller(target, root) — wheel-ups inside an expanded tool
// output's own scroller must not drop the transcript pin.
describe("insideNestedScroller", () => {
  function chain() {
    const root = document.createElement("div");
    const box = document.createElement("div");
    const target = document.createElement("span");
    box.appendChild(target);
    root.appendChild(box);
    return { root, box, target };
  }

  it("holds when an inner scroller has scrollback of its own", () => {
    const { root, box, target } = chain();
    // jsdom doesn't clamp; a real browser only allows this on a scroller.
    box.scrollTop = 5;
    expect(insideNestedScroller(target, root)).toBe(true);
  });

  it("holds for an overflowing scroll container even at its top", () => {
    const { root, box, target } = chain();
    box.style.overflowY = "auto";
    Object.defineProperty(box, "scrollHeight", { value: 300 });
    Object.defineProperty(box, "clientHeight", { value: 100 });
    expect(insideNestedScroller(target, root)).toBe(true);
  });

  it("passes plain rows through (no scroller between target and root)", () => {
    const { root, target } = chain();
    expect(insideNestedScroller(target, root)).toBe(false);
  });

  it("ignores scroll containers whose content does not overflow", () => {
    const { root, box, target } = chain();
    box.style.overflowY = "auto";
    Object.defineProperty(box, "scrollHeight", { value: 100 });
    Object.defineProperty(box, "clientHeight", { value: 100 });
    expect(insideNestedScroller(target, root)).toBe(false);
  });

  it("never counts the transcript root itself", () => {
    const { root, target } = chain();
    root.scrollTop = 500;
    expect(insideNestedScroller(target, root)).toBe(false);
  });
});
