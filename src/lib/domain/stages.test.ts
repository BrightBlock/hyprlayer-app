import { describe, expect, it } from "vitest";

import {
  MODE_STAGES,
  STAGES,
  nextModeStage,
  stageCommand,
  suggestedDocKind,
} from "./stages";

describe("stage modes", () => {
  it("excludes ship from the selectable modes but keeps it as a known stage", () => {
    expect(MODE_STAGES.map((s) => s.key)).not.toContain("ship");
    expect(STAGES.map((s) => s.key)).toContain("ship");
    // Every selectable mode is still a real stage.
    for (const mode of MODE_STAGES) {
      expect(STAGES.find((s) => s.key === mode.key)).toBeDefined();
    }
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

describe("stageCommand", () => {
  it("maps spec stages to their skill slash-command", () => {
    expect(stageCommand("research")).toBe("/research_codebase");
    expect(stageCommand("plan")).toBe("/create_plan");
    expect(stageCommand("implement")).toBe("/implement_plan");
    expect(stageCommand("validate")).toBe("/validate_plan");
    expect(stageCommand("commit")).toBe("/commit");
  });

  it("has no command for plain-prompt stages", () => {
    expect(stageCommand("general")).toBeNull();
    expect(stageCommand("ship")).toBeNull();
    expect(stageCommand("revise")).toBeNull();
    expect(stageCommand("nonsense")).toBeNull();
  });
});

// Drives the "Cycle Agent Stage" keybinding (default Shift+Tab).
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
    // commit/ship/revise are real stages but excluded from the mode picker; an
    // unknown string is treated the same — both start the cycle at general.
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
    expect(cur).toBe(MODE_STAGES[0].key); // full loop returns to the start
  });
});
