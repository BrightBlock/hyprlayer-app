import { describe, expect, it } from "vitest";

import { isNewerVersion } from "./updater";

// This is what drives the Linux update nudge: there is no signed manifest entry
// for Linux, so the changelog's newest version vs. the running one is the only
// signal. A false positive nags forever, so the malformed cases matter.
describe("isNewerVersion", () => {
  it("compares each component numerically, not lexically", () => {
    expect(isNewerVersion("0.5.10", "0.5.9")).toBe(true);
    expect(isNewerVersion("0.10.0", "0.9.9")).toBe(true);
    expect(isNewerVersion("1.0.0", "0.99.99")).toBe(true);
  });

  it("is false for equal and older versions", () => {
    expect(isNewerVersion("0.5.7", "0.5.7")).toBe(false);
    expect(isNewerVersion("0.5.6", "0.5.7")).toBe(false);
    expect(isNewerVersion("0.4.9", "0.5.0")).toBe(false);
  });

  it("refuses anything that isn't a strict x.y.z", () => {
    expect(isNewerVersion("0.5.8-rc.1", "0.5.7")).toBe(false);
    expect(isNewerVersion("0.6", "0.5.7")).toBe(false);
    expect(isNewerVersion("v0.5.8", "0.5.7")).toBe(false);
    expect(isNewerVersion("0.5.8", "")).toBe(false);
  });
});
