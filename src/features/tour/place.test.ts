import { describe, expect, it } from "vitest";

import { isVisible, placeCoach } from "./place";

const view = { width: 1440, height: 900 };
const panel = { width: 344, height: 220 };

describe("placeCoach", () => {
  it("sits above a bottom-docked anchor with the caret over its centre", () => {
    const p = placeCoach({ left: 600, top: 800, width: 200, height: 40 }, panel, "top", view);
    expect(p.side).toBe("top");
    expect(p.top + panel.height).toBeLessThanOrEqual(800);
    expect(p.left + p.caret + 4.5).toBeCloseTo(700, 0);
  });

  it("flips to the other side when the preferred one has no room", () => {
    const p = placeCoach({ left: 600, top: 20, width: 200, height: 24 }, panel, "top", view);
    expect(p.side).toBe("bottom");
    expect(p.top).toBeGreaterThanOrEqual(44);
  });

  it("goes left of a right-hand panel and stays inside the window", () => {
    const p = placeCoach({ left: 1100, top: 100, width: 340, height: 120 }, panel, "left", view);
    expect(p.side).toBe("left");
    expect(p.left + panel.width).toBeLessThanOrEqual(1100);
    expect(p.top).toBeGreaterThanOrEqual(8);
  });

  it("clamps into the window when no side fits", () => {
    const p = placeCoach({ left: 0, top: 0, width: 1440, height: 900 }, panel, "top", view);
    expect(p.left).toBeGreaterThanOrEqual(8);
    expect(p.top).toBeGreaterThanOrEqual(8);
  });
});

describe("isVisible", () => {
  it("rejects a collapsed or off-screen rect", () => {
    expect(isVisible({ left: 10, top: 10, width: 0, height: 20 }, view)).toBe(false);
    expect(isVisible({ left: 2000, top: 10, width: 20, height: 20 }, view)).toBe(false);
    expect(isVisible({ left: 10, top: 10, width: 20, height: 20 }, view)).toBe(true);
  });
});
