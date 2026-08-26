import { describe, expect, it } from "vitest";

import type { AppSettings, CustomPrompt, PromptContext, PromptDefaults } from "@/lib/bindings";
import {
  bodyHasVars,
  effectiveBuiltinBody,
  effectiveCustomBody,
  isBuiltinModified,
  matchComposerPrompt,
  offeredPrompts,
  pendingComposerArg,
  prefillSlashCommand,
  promptFirstLine,
  promptInputs,
  promptSlug,
  resolvePromptBody,
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

describe("promptInputs", () => {
  it("finds bare and labelled slots in order of appearance", () => {
    expect(promptInputs("Migrate {{input:from}} to {{input:to}}, then {{input}}")).toEqual([
      { token: "{{input:from}}", label: "from" },
      { token: "{{input:to}}", label: "to" },
      { token: "{{input}}", label: "" },
    ]);
  });

  it("collapses a repeated token to one field and trims the label", () => {
    expect(promptInputs("Review {{input: which file }}; then re-run {{input: which file }}")).toEqual(
      [{ token: "{{input: which file }}", label: "which file" }],
    );
  });

  it("ignores lookalikes and bodies with no slot", () => {
    expect(promptInputs("{{inputs}} {{branch}} plain text")).toEqual([]);
  });

  it("treats an empty label as unlabelled", () => {
    expect(promptInputs("{{input:}}")).toEqual([{ token: "{{input:}}", label: "" }]);
  });
});

describe("resolvePromptBody", () => {
  it("substitutes variables and arguments together", () => {
    expect(
      resolvePromptBody("On {{branch}}: fix {{input:what}}", CTX, { "{{input:what}}": "the tabs" }),
    ).toBe("On feature/x: fix the tabs");
  });

  it("fills every occurrence of a repeated token from one answer", () => {
    expect(resolvePromptBody("{{input}} then {{input}}", null, { "{{input}}": "go" })).toBe(
      "go then go",
    );
  });

  it("resolves an unanswered slot to empty", () => {
    expect(resolvePromptBody("run {{input:x}} now", null, {})).toBe("run  now");
  });

  it("leaves variables verbatim when no context was fetched", () => {
    expect(resolvePromptBody("on {{branch}}", null, {})).toBe("on {{branch}}");
  });

  it("never expands a variable token the user typed as an argument", () => {
    expect(
      resolvePromptBody("Compare {{input:a}} with {{diff}}", CTX, { "{{input:a}}": "{{diff}}" }),
    ).toBe("Compare {{diff}} with diff --git a b");
  });

  it("never expands an argument token that came out of a variable value", () => {
    const tricky = { ...CTX, diff: "+ ask({{input}})" };
    expect(resolvePromptBody("{{diff}}, {{input}}", tricky, { "{{input}}": "GO" })).toBe(
      "+ ask({{input}}), GO",
    );
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
    expect(effectiveBuiltinBody(s, DEFAULTS, "ship")).toBe(DEFAULTS.ship);
    expect(isBuiltinModified(s, DEFAULTS, "ship")).toBe(false);
  });

  it("an override equal to stock does not read as modified", () => {
    const s = settingsWith({ builtinBodies: { commit: "/commit" } });
    expect(isBuiltinModified(s, DEFAULTS, "commit")).toBe(false);
  });
});

describe("promptSlug", () => {
  it("turns a name into a /command token", () => {
    expect(promptSlug("Review file")).toBe("review-file");
    expect(promptSlug("Ship it!")).toBe("ship-it");
    expect(promptSlug("  Fix   the   tests  ")).toBe("fix-the-tests");
  });

  it("is empty when the name has nothing usable", () => {
    expect(promptSlug("···")).toBe("");
    expect(promptSlug("")).toBe("");
  });
});

describe("offeredPrompts", () => {
  const NONE = new Set<string>();

  it("keys runnable prompts by slug", () => {
    const p: CustomPrompt = { id: "p1", name: "Review file", body: "Review {{input}}" };
    expect([...offeredPrompts([p], null, NONE).keys()]).toEqual(["review-file"]);
  });

  it("drops a slug a real claude command already owns", () => {
    const p: CustomPrompt = { id: "p1", name: "Review", body: "Review it" };
    expect(offeredPrompts([p], null, new Set(["review"])).size).toBe(0);
  });

  it("drops an unusable name and an empty body for this project", () => {
    const emoji: CustomPrompt = { id: "p1", name: "✨", body: "text" };
    const blank: CustomPrompt = { id: "p2", name: "Blank", body: "   " };
    const overridden: CustomPrompt = { id: "p3", name: "Scoped", body: "global", overrides: { "/a": "" } };
    expect(offeredPrompts([emoji, blank, overridden], "/a", NONE).size).toBe(0);
    expect([...offeredPrompts([overridden], "/b", NONE).keys()]).toEqual(["scoped"]);
  });

  it("resolves a duplicate slug to the prompt that was actually offered", () => {
    const empty: CustomPrompt = { id: "p1", name: "Review", body: "" };
    const real: CustomPrompt = { id: "p2", name: "Review", body: "Review it" };
    expect(offeredPrompts([empty, real], null, NONE).get("review")).toBe(real);
  });

  it("keeps the first of two runnable duplicates", () => {
    const a: CustomPrompt = { id: "p1", name: "Review", body: "first" };
    const b: CustomPrompt = { id: "p2", name: "Review", body: "second" };
    expect(offeredPrompts([a, b], null, NONE).get("review")).toBe(a);
  });
});

describe("matchComposerPrompt", () => {
  const review: CustomPrompt = { id: "p1", name: "Review file", body: "Review {{input}}" };
  const offered = new Map([["review-file", review]]);

  it("matches a leading slug and takes the rest as the argument", () => {
    expect(matchComposerPrompt("/review-file src/foo.ts", offered)).toEqual({
      prompt: review,
      typed: "src/foo.ts",
    });
  });

  it("matches with nothing typed after it", () => {
    expect(matchComposerPrompt("/review-file", offered)).toEqual({
      prompt: review,
      typed: "",
    });
  });

  it("takes everything after the slug, newlines included", () => {
    expect(matchComposerPrompt("/review-file a.ts\nand b.ts", offered)?.typed).toBe(
      "a.ts\nand b.ts",
    );
  });

  it("ignores a slug that is not the draft's first token", () => {
    expect(matchComposerPrompt("please /review-file a.ts", offered)).toBeNull();
  });

  it("ignores a slug a real claude command already owns", () => {
    expect(matchComposerPrompt("/review-file a.ts", new Map())).toBeNull();
  });

  it("ignores an unknown command", () => {
    expect(matchComposerPrompt("/security-review", offered)).toBeNull();
  });
});

describe("pendingComposerArg", () => {
  const one: CustomPrompt = { id: "p1", name: "Explain", body: "Explain {{input}} simply" };
  const labelled: CustomPrompt = { id: "p2", name: "Scope", body: "{{input:which module}}" };
  const two: CustomPrompt = { id: "p3", name: "Pair", body: "{{input:a}} then {{input:b}}" };
  const none: CustomPrompt = { id: "p4", name: "Plain", body: "No slots here" };

  it("names the lone unfilled argument, with its label when it has one", () => {
    expect(pendingComposerArg({ prompt: one, typed: "" }, null)?.label).toBe("");
    expect(pendingComposerArg({ prompt: labelled, typed: "" }, null)?.label).toBe("which module");
  });

  it("is clear once the argument is typed", () => {
    expect(pendingComposerArg({ prompt: one, typed: "closures" }, null)).toBeNull();
  });

  it("leaves multi-slot and slot-free bodies to the dialog and a direct send", () => {
    expect(pendingComposerArg({ prompt: two, typed: "" }, null)).toBeNull();
    expect(pendingComposerArg({ prompt: none, typed: "" }, null)).toBeNull();
  });

  it("reads the slots from this project's override", () => {
    const scoped: CustomPrompt = { id: "p5", name: "Scoped", body: "no slots", overrides: { "/a": "{{input:pick}}" } };
    expect(pendingComposerArg({ prompt: scoped, typed: "" }, "/a")?.label).toBe("pick");
    expect(pendingComposerArg({ prompt: scoped, typed: "" }, "/b")).toBeNull();
  });
});

describe("prefillSlashCommand", () => {
  it("fills an empty draft with the command and a trailing space", () => {
    expect(prefillSlashCommand("", "review-file")).toBe("/review-file ");
  });

  it("keeps what was already typed as the argument", () => {
    expect(prefillSlashCommand("src/foo.ts", "review-file")).toBe("/review-file src/foo.ts");
  });

  it("swaps a command already at the head instead of nesting one", () => {
    expect(prefillSlashCommand("/ship-it now", "review-file")).toBe("/review-file now");
    expect(prefillSlashCommand("/ship-it", "review-file")).toBe("/review-file ");
  });
});

describe("promptFirstLine", () => {
  it("returns the first line only", () => {
    expect(promptFirstLine("one\ntwo")).toBe("one");
    expect(promptFirstLine("single")).toBe("single");
    expect(promptFirstLine("")).toBe("");
  });
});
