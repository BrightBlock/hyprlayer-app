import { describe, expect, it } from "vitest";

import {
  MODE_STAGES,
  SHIP_PROMPT,
  STAGES,
  composeStagePrompt,
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
  });

  it("has no command for plain-prompt stages", () => {
    expect(stageCommand("general")).toBeNull();
    expect(stageCommand("ship")).toBeNull();
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
    expect(composeStagePrompt("ship", "")).toBe(SHIP_PROMPT);
  });

  it("appends extra ship context below the directive", () => {
    const prompt = composeStagePrompt("ship", "skip the changelog");
    expect(prompt).toBe(`${SHIP_PROMPT}\n\nskip the changelog`);
  });
});
