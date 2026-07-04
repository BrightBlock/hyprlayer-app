import { describe, expect, it } from "vitest";

import type { AppSettings, CustomPrompt, PromptContext, PromptDefaults } from "../../lib/bindings";
import {
  bodyHasVars,
  effectiveBuiltinBody,
  effectiveCustomBody,
  isBuiltinModified,
  promptFirstLine,
  substitutePromptVars,
} from "./prompts";

const CTX: PromptContext = {
  branch: "feature/x",
  baseBranch: "main",
  changedFiles: "4 files (+182 −40)",
  diff: "diff --git a b",
  lastCommit: "fix(tabs): debounce",
};

const DEFAULTS: PromptDefaults = {
  commit: "/commit",
  ship: "Push and open a PR against `{{base_branch}}`.",
};

function settingsWith(prompts: AppSettings["prompts"]): AppSettings {
  return { prompts } as AppSettings;
}

describe("substitutePromptVars", () => {
  it("replaces every known token, matching the backend rule", () => {
    expect(
      substitutePromptVars("On {{branch}} → {{base_branch}}: {{changed_files}}", CTX),
    ).toBe("On feature/x → main: 4 files (+182 −40)");
    expect(substitutePromptVars("{{last_commit}}\n{{diff}}", CTX)).toBe(
      "fix(tabs): debounce\ndiff --git a b",
    );
  });

  it("replaces repeated tokens and leaves unknown tokens verbatim", () => {
    expect(substitutePromptVars("{{branch}} {{branch}}", CTX)).toBe("feature/x feature/x");
    expect(substitutePromptVars("{{ticket}} on {{branch}}", CTX)).toBe(
      "{{ticket}} on feature/x",
    );
  });

  it("never re-substitutes token-shaped text inside a substituted value", () => {
    // A diff hunk may legitimately contain another variable's token; single-pass
    // substitution (matching the backend) must leave it verbatim.
    const tricky = { ...CTX, diff: '+ body.contains("{{last_commit}}")' };
    expect(substitutePromptVars("{{diff}}", tricky)).toBe(
      '+ body.contains("{{last_commit}}")',
    );
  });
});

describe("bodyHasVars", () => {
  it("detects known tokens only", () => {
    expect(bodyHasVars("run on {{branch}}")).toBe(true);
    expect(bodyHasVars("nothing here")).toBe(false);
    expect(bodyHasVars("{{unknown}} token")).toBe(false);
  });
});

describe("effectiveCustomBody", () => {
  const prompt: CustomPrompt = {
    id: "p1",
    name: "Tests",
    body: "global body",
    overrides: { "/repos/app": "app override" },
  };

  it("prefers the project's override, falling back to the global body", () => {
    expect(effectiveCustomBody(prompt, "/repos/app")).toBe("app override");
    expect(effectiveCustomBody(prompt, "/repos/other")).toBe("global body");
    expect(effectiveCustomBody(prompt, null)).toBe("global body");
  });

  it("tolerates sparse prompts", () => {
    expect(effectiveCustomBody({ id: "x" }, "/repos/app")).toBe("");
  });
});

describe("effectiveBuiltinBody / isBuiltinModified", () => {
  it("uses stock text when no override exists", () => {
    const s = settingsWith({});
    expect(effectiveBuiltinBody(s, DEFAULTS, "commit")).toBe("/commit");
    expect(isBuiltinModified(s, DEFAULTS, "commit")).toBe(false);
  });

  it("uses the override when set and reports modified", () => {
    const s = settingsWith({ builtinBodies: { commit: "/commit --strict" } });
    expect(effectiveBuiltinBody(s, DEFAULTS, "commit")).toBe("/commit --strict");
    expect(isBuiltinModified(s, DEFAULTS, "commit")).toBe(true);
    // Ship is untouched by the commit override.
    expect(effectiveBuiltinBody(s, DEFAULTS, "ship")).toBe(DEFAULTS.ship);
    expect(isBuiltinModified(s, DEFAULTS, "ship")).toBe(false);
  });

  it("an override equal to stock does not read as modified", () => {
    const s = settingsWith({ builtinBodies: { commit: "/commit" } });
    expect(isBuiltinModified(s, DEFAULTS, "commit")).toBe(false);
  });
});

describe("promptFirstLine", () => {
  it("returns the first line only", () => {
    expect(promptFirstLine("one\ntwo")).toBe("one");
    expect(promptFirstLine("single")).toBe("single");
    expect(promptFirstLine("")).toBe("");
  });
});
