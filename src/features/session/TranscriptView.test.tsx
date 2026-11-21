// @vitest-environment jsdom
import { createRef } from "react";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  TranscriptView,
  insideNestedScroller,
  releaseOnWheel,
  stickAfterScroll,
  type TranscriptHandle,
} from "./TranscriptView";

describe("stickAfterScroll", () => {
  it("releases on a genuine scroll-up (scrollTop dropped, real gap opened)", () => {
    expect(stickAfterScroll(1000, 500, 500, true)).toBe(false);
  });

  it("holds the stick when the viewport grows (composer shrank) and scrollTop clamps", () => {
    expect(stickAfterScroll(1000, 960, 0, true)).toBe(true);
  });

  it("holds the stick when content shrinks and scrollTop clamps (gap ~0)", () => {
    expect(stickAfterScroll(1000, 999, 1, true)).toBe(true);
  });

  it("does not release on streaming growth (gap opens but scrollTop did not drop)", () => {
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
    expect(releaseOnWheel(0, -40, false, 0)).toBe(false);
  });
});

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

describe("re-pinning on resize", () => {
  let resizes: ResizeObserverCallback[] = [];
  let frames: FrameRequestCallback[] = [];

  beforeEach(() => {
    resizes = [];
    frames = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: ResizeObserverCallback) {
          resizes.push(cb);
        }
        observe() {}
        disconnect() {}
      },
    );
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal("cancelAnimationFrame", () => {});
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function renderStuck(busy = false) {
    const { container } = render(
      <TranscriptView
        items={[]}
        busy={busy}
        permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
        prompt={{ onRewind: () => {} }}
      />,
    );
    const el = container.firstChild as HTMLDivElement;
    let clientHeight = 400;
    Object.defineProperty(el, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
    el.scrollTop = 600;
    return { el, shrinkViewportTo: (h: number) => (clientHeight = h) };
  }

  const resize = () => {
    resizes.forEach((cb) => cb([], {} as ResizeObserver));
  };

  it("re-pins to the new bottom when the viewport shrinks", () => {
    const { el, shrinkViewportTo } = renderStuck();
    shrinkViewportTo(300);
    resize();
    expect(el.scrollTop).toBe(700);
  });

  it("leaves a user who scrolled up where they are", () => {
    const { el, shrinkViewportTo } = renderStuck();
    fireEvent.wheel(el, { deltaY: -40 });
    el.scrollTop = 200;
    shrinkViewportTo(300);
    resize();
    expect(el.scrollTop).toBe(200);
  });

  it("does not start a second animation loop while a turn is busy", () => {
    const { el, shrinkViewportTo } = renderStuck(true);
    expect(frames).toHaveLength(1);

    shrinkViewportTo(300);
    resize();

    expect(el.scrollTop).toBe(700);
    expect(frames).toHaveLength(1);
  });
});

describe("jumping to an item the view has not read in yet", () => {
  let frames: FrameRequestCallback[] = [];
  const flush = () => {
    for (let i = 0; i < 50 && frames.length > 0; i += 1) frames.splice(0).forEach((cb) => cb(0));
  };

  beforeEach(() => {
    frames = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        disconnect() {}
      },
    );
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.stubGlobal("CSS", { escape: (value: string) => value });
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("waits for the older turns, then scrolls to it", () => {
    const scrolled = vi.fn();
    Element.prototype.scrollIntoView = scrolled;
    const newer = { kind: "error" as const, id: "i1", detail: "newer", raw: "" };
    const older = { kind: "error" as const, id: "i9", detail: "older", raw: "" };
    const ref = createRef<TranscriptHandle>();
    const view = (items: (typeof newer)[], olderTurns: "loading" | undefined) => (
      <TranscriptView
        ref={ref}
        items={items}
        busy={false}
        olderTurns={olderTurns}
        permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
        prompt={{ onRewind: () => {} }}
      />
    );
    const { rerender } = render(view([newer], "loading"));

    act(() => ref.current!.scrollToItem("i9"));
    act(flush);
    rerender(view([older, newer], undefined));
    act(flush);

    expect(scrolled).toHaveBeenCalledTimes(1);
    expect((scrolled.mock.contexts[0] as Element).getAttribute("data-item-id")).toBe("i9");
  });

  it("holds a reader scrolled up still when the older turns land, though the first row folds away", () => {
    const stub = { kind: "error" as const, id: "i1", detail: "stand-in", raw: "" };
    const older = { kind: "error" as const, id: "i7", detail: "older", raw: "" };
    const newer = { kind: "error" as const, id: "i2", detail: "newer", raw: "" };
    const view = (items: (typeof stub)[], olderTurns: "loading" | undefined) => (
      <TranscriptView
        items={items}
        busy={false}
        olderTurns={olderTurns}
        permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
        prompt={{ onRewind: () => {} }}
      />
    );
    const { container, rerender } = render(view([stub, newer], "loading"));
    const el = container.firstChild as HTMLDivElement;
    // The older row adds 600px once it is in the DOM.
    const height = () => 1000 + (el.querySelector('[data-item-id="i7"]') ? 600 : 0);
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: height });
    Object.defineProperty(el, "clientHeight", { configurable: true, value: 400 });
    el.scrollTop = 200;
    fireEvent.wheel(el, { deltaY: -40 });

    rerender(view([older, newer], undefined));

    expect(el.scrollTop).toBe(800);
  });

  it("offers to load the older turns again when reading them failed", () => {
    const load = vi.fn();
    const { getByText, getByRole } = render(
      <TranscriptView
        items={[{ kind: "error", id: "i1", detail: "newer", raw: "" }]}
        busy={false}
        olderTurns="failed"
        onLoadOlder={load}
        permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
        prompt={{ onRewind: () => {} }}
      />,
    );

    expect(getByText("Earlier turns didn't load")).toBeTruthy();
    fireEvent.click(getByRole("button", { name: "Load earlier turns" }));
    expect(load).toHaveBeenCalledTimes(1);
  });
});
