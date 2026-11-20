import { describe, expect, it } from "vitest";

import { retryWrites } from "./retryPick";

// What the footer resolves to from this device's defaults when the tab holds no pick of its own.
const DEFAULTS = { model: "claude-opus-5", effort: "xhigh" };

describe("what Resume writes for a start that failed", () => {
  it("writes nothing for a tab with no pick of its own, whatever the device defaults are", () => {
    for (const harness of ["claude", "codex", "opencode"] as const) {
      expect(retryWrites(harness, {}, DEFAULTS)).toEqual({ model: null, effort: null });
    }
  });

  it("writes the model and effort the user picked on this tab", () => {
    expect(retryWrites("claude", { model: "sonnet", effort: "high" }, { model: "sonnet-flag", effort: "high" })).toEqual({
      model: "sonnet-flag",
      effort: "high",
    });
    expect(
      retryWrites("codex", { codexModel: "gpt-6-sol", codexEffort: "low" }, { model: "gpt-6-sol", effort: "low" }),
    ).toEqual({ model: "gpt-6-sol", effort: "low" });
  });

  it("writes only the half the user picked", () => {
    expect(retryWrites("claude", { model: "sonnet" }, { model: "sonnet-flag", effort: "xhigh" })).toEqual({
      model: "sonnet-flag",
      effort: null,
    });
    expect(retryWrites("opencode", { opencodeModel: "opencode/mimo-free" }, { model: "opencode/mimo-free", effort: null })).toEqual({
      model: "opencode/mimo-free",
      effort: null,
    });
  });

  it("ignores another harness's pick", () => {
    expect(retryWrites("opencode", { model: "sonnet", effort: "high" }, DEFAULTS)).toEqual({ model: null, effort: null });
  });
});
