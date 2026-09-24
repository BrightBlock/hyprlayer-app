import { describe, expect, it } from "vitest";

import {
  FOLLOW_HARNESS,
  titleModelFromValue,
  titleModelGroups,
  titleModelValue,
} from "./titleModel";

const CLAUDE = [
  { value: "claude:haiku", label: "Haiku 4.5" },
  { value: "claude:sonnet", label: "Sonnet 5" },
];

describe("title model value mapping", () => {
  it("selects Follow when nothing is pinned", () => {
    expect(titleModelValue(null)).toBe(FOLLOW_HARNESS);
    expect(titleModelValue(undefined)).toBe(FOLLOW_HARNESS);
    expect(titleModelValue("  ")).toBe(FOLLOW_HARNESS);
  });

  it("keeps a pinned model", () => {
    expect(titleModelValue("claude:sonnet")).toBe("claude:sonnet");
  });

  it("writes null for Follow, so the harness default keeps following", () => {
    expect(titleModelFromValue(FOLLOW_HARNESS)).toBeNull();
    expect(titleModelFromValue("codex:gpt-5.4-mini")).toBe("codex:gpt-5.4-mini");
  });
});

describe("titleModelGroups", () => {
  it("offers Follow first, short enough for the 190px select", () => {
    const groups = titleModelGroups({ claude: CLAUDE, codex: [], opencode: [], pinned: null, defaultHarness: null });
    expect(groups[0].options).toEqual([{ value: FOLLOW_HARNESS, label: "Follow harness" }]);
  });

  it("marks the model Follow resolves to as the default, per harness", () => {
    const label = (groups: ReturnType<typeof titleModelGroups>, value: string) =>
      groups.flatMap((g) => g.options).find((o) => o.value === value)?.label;
    const claude = titleModelGroups({ claude: CLAUDE, codex: [], opencode: [], pinned: null, defaultHarness: null });
    expect(label(claude, "claude:haiku")).toBe("Haiku 4.5 (default)");
    expect(label(claude, "claude:sonnet")).toBe("Sonnet 5");
    const codex = titleModelGroups({ claude: CLAUDE, codex: [], opencode: [], pinned: null, defaultHarness: "codex" });
    expect(label(codex, "codex:gpt-5.4-mini")).toBe("gpt-5.4-mini (default)");
    expect(label(codex, "claude:haiku")).toBe("Haiku 4.5");
  });

  it("always offers the codex title default, without duplicating a selected one", () => {
    const groups = titleModelGroups({
      claude: CLAUDE,
      codex: ["gpt-5.4-mini", "gpt-5.5"],
      opencode: [],
      pinned: null,
      defaultHarness: null,
    });
    const codex = groups.find((g) => g.label === "Codex")!;
    expect(codex.options.map((o) => o.value)).toEqual(["codex:gpt-5.4-mini", "codex:gpt-5.5"]);
  });

  it("adds no group for a pin the catalogs already list", () => {
    const groups = titleModelGroups({ claude: CLAUDE, codex: [], opencode: [], pinned: "claude:sonnet", defaultHarness: null });
    expect(groups.map((g) => g.label)).not.toContain("Pinned");
  });

  it("keeps a pin no catalog lists, so the select never shows a different model", () => {
    const groups = titleModelGroups({
      claude: CLAUDE,
      codex: [],
      opencode: [{ value: "opencode:acme/tiny", label: "Tiny" }],
      pinned: "opencode:retired/model",
      defaultHarness: null,
    });
    const pinned = groups.find((g) => g.label === "Pinned");
    expect(pinned?.options).toEqual([{ value: "opencode:retired/model", label: "opencode:retired/model" }]);
  });

  it("omits empty groups", () => {
    const groups = titleModelGroups({ claude: CLAUDE, codex: [], opencode: [], pinned: null, defaultHarness: null });
    expect(groups.map((g) => g.label)).toEqual(["", "Claude", "Codex"]);
  });
});
