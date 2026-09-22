import { describe, expect, it } from "vitest";

import {
  MODE_STAGES,
  STAGES,
  nextModeStage,
  suggestedDocKind,
} from "./stages";

describe("stage modes", () => {
  it("excludes ship from the selectable modes but keeps it as a known stage", () => {
    expect(MODE_STAGES.map((s) => s.key)).not.toContain("ship");
    expect(STAGES.map((s) => s.key)).toContain("ship");
  });
});

describe("suggestedDocKind", () => {
  it("plan stage suggests research docs", () => {
    expect(suggestedDocKind("plan")).toBe("research");
  });

  it("implement stage suggests plan docs", () => {
    expect(suggestedDocKind("implement")).toBe("plan");
  });

  it("validate stage suggests plan docs", () => {
    expect(suggestedDocKind("validate")).toBe("plan");
  });

  it("general and research stages suggest nothing", () => {
    expect(suggestedDocKind("general")).toBeNull();
    expect(suggestedDocKind("research")).toBeNull();
  });

  it("unknown stages suggest nothing", () => {
    expect(suggestedDocKind("nonsense")).toBeNull();
    expect(suggestedDocKind("")).toBeNull();
  });
});

describe("nextModeStage", () => {
  it("advances through the selectable stages in order", () => {
    expect(nextModeStage("general")).toBe("research");
    expect(nextModeStage("research")).toBe("plan");
    expect(nextModeStage("plan")).toBe("implement");
    expect(nextModeStage("implement")).toBe("validate");
  });

  it("wraps the last selectable stage back to the first", () => {
    expect(nextModeStage("validate")).toBe("general");
  });

  it("starts at the first mode for stages outside the cycle", () => {
    for (const stage of ["commit", "ship", "revise", "bogus"]) {
      expect(nextModeStage(stage)).toBe("general");
    }
  });

  it("visits every selectable stage exactly once before wrapping", () => {
    const seen = new Set<string>();
    let cur: string = MODE_STAGES[0].key;
    for (let n = 0; n < MODE_STAGES.length; n++) {
      seen.add(cur);
      cur = nextModeStage(cur);
    }
    expect(seen.size).toBe(MODE_STAGES.length);
    expect(cur).toBe(MODE_STAGES[0].key);
  });
});
