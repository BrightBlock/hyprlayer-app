import { describe, expect, it } from "vitest";

import {
  MODE_STAGES,
  STAGES,
  composeStagePrompt,
  nextModeStage,
  shipPrompt,
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

// These mirror the Rust unit tests for `compose_prompt`
// (crates/hyprlayer-core/src/pipeline.rs); keep both in sync.
describe("composeStagePrompt", () => {
  it("passes general input through verbatim (trimmed)", () => {
    expect(composeStagePrompt("general", "  hello  ")).toBe("hello");
  });

  it("prepends the skill command for spec stages", () => {
    expect(composeStagePrompt("research", "how does auth work?")).toBe(
      "/research_codebase how does auth work?",
    );
  });

  it("invokes the bare skill when input is empty", () => {
    expect(composeStagePrompt("research", "")).toBe("/research_codebase");
  });

  it("ships with the standalone directive when there is no input", () => {
    expect(composeStagePrompt("ship", "", "main")).toBe(shipPrompt("main"));
  });

  it("names the target branch in the ship directive (git-flow re-targeting)", () => {
    expect(composeStagePrompt("ship", "", "develop")).toContain("`develop`");
  });

  // Golden literal — must stay byte-identical to the Rust `ship_prompt` test
  // (crates/hyprlayer-core/src/pipeline.rs). Editing one side fails its golden,
  // forcing both to move together so the optimistic echo can never drift.
  it("renders the exact ship directive (golden, matches the backend)", () => {
    expect(shipPrompt("main")).toBe(
      "First commit all changes in this worktree, then push the branch to its remote. " +
        "Only after the commit and push succeed, open a pull request against `main` with a " +
        "clear title and run the /describe_pr skill to write the PR description from the " +
        "repository's template and update the PR.",
    );
  });

  it("strips the origin/ prefix so the PR targets the branch, not the rev", () => {
    expect(shipPrompt("origin/develop")).toBe(shipPrompt("develop"));
    expect(shipPrompt("origin/main")).toContain("`main`");
  });

  it("appends extra ship context below the directive", () => {
    const prompt = composeStagePrompt("ship", "skip the changelog", "main");
    expect(prompt).toBe(`${shipPrompt("main")}\n\nskip the changelog`);
  });

  it("commit invokes the /commit skill with any input as its argument", () => {
    expect(composeStagePrompt("commit", "")).toBe("/commit");
    expect(composeStagePrompt("commit", "split into two commits")).toBe(
      "/commit split into two commits",
    );
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
