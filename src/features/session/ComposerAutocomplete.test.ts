import { describe, expect, it } from "vitest";

import type { SlashCommand } from "@/lib/bindings";

import {
  detectTrigger,
  highlightSegments,
  isClaudeFamilyModel,
  resolvesInHarness,
  skillSigilFor,
} from "./ComposerAutocomplete";

describe("detectTrigger", () => {
  it("detects a command at the start of the message", () => {
    const t = detectTrigger("/rese", 5);
    expect(t).toEqual({ type: "command", start: 0, query: "rese" });
  });

  it("detects a slash mid-prompt as a command token", () => {
    const text = "look at /research";
    expect(detectTrigger(text, text.length)).toEqual({
      type: "command",
      start: 8,
      query: "research",
    });
  });

  it("does not treat a slash inside a token as a command", () => {
    const text = "src/main";
    expect(detectTrigger(text, text.length)).toBeNull();
  });

  it("allows leading whitespace before a command", () => {
    const t = detectTrigger("  /plan", 7);
    expect(t).toEqual({ type: "command", start: 2, query: "plan" });
  });

  it("detects an @file token anywhere", () => {
    const text = "see @src/main";
    const t = detectTrigger(text, text.length);
    expect(t).toEqual({ type: "file", start: 4, query: "src/main" });
  });

  it("does not treat an email-like @ as a file token", () => {
    const text = "ping me@host";
    expect(detectTrigger(text, text.length)).toBeNull();
  });

  it("closes once the token ends in whitespace", () => {
    const text = "/research_codebase ";
    expect(detectTrigger(text, text.length)).toBeNull();
  });
});

describe("highlightSegments", () => {
  const known = new Set(["research_codebase", "code-review"]);

  it("marks a recognized skill token", () => {
    const segs = highlightSegments("run /research_codebase now", known);
    expect(segs.filter((s) => s.skill).map((s) => s.text)).toEqual([
      "/research_codebase",
    ]);
  });

  it("leaves unknown slash tokens unhighlighted", () => {
    const segs = highlightSegments("/not_a_skill", known);
    expect(segs.some((s) => s.skill)).toBe(false);
  });

  it("highlights a skill referenced mid-prompt", () => {
    const segs = highlightSegments("first do /code-review please", known);
    expect(segs.filter((s) => s.skill).map((s) => s.text)).toEqual(["/code-review"]);
  });

  it("paints the harness's own sigil", () => {
    const segs = highlightSegments("run $research_codebase now", known, "$");
    expect(segs.filter((s) => s.skill).map((s) => s.text)).toEqual(["$research_codebase"]);
  });

  it("leaves the other harness's sigil alone", () => {
    expect(highlightSegments("run /research_codebase", known, "$").some((s) => s.skill)).toBe(
      false,
    );
    expect(highlightSegments("run $research_codebase", known, "/").some((s) => s.skill)).toBe(
      false,
    );
  });
});

describe("skillSigilFor", () => {
  it("uses Codex's own explicit-invocation sigil on codex", () => {
    expect(skillSigilFor("codex")).toBe("$");
  });

  it("keeps Claude's slash on Claude and Claude-family OpenCode models", () => {
    expect(skillSigilFor("claude")).toBe("/");
    expect(skillSigilFor("opencode", "anthropic/claude-sonnet-4-6")).toBe("/");
    expect(skillSigilFor("opencode", "openrouter/anthropic/claude-opus-4.1")).toBe("/");
  });

  it("uses Codex behavior for every other OpenCode model", () => {
    expect(skillSigilFor("opencode", "openai/gpt-5.4")).toBe("$");
    expect(skillSigilFor("opencode", "google/gemini-3-pro")).toBe("$");
    expect(skillSigilFor("opencode", "ollama/qwen3-coder")).toBe("$");
    expect(skillSigilFor("opencode", null)).toBe("$");
  });
});

describe("isClaudeFamilyModel", () => {
  it("recognizes Claude through first-party and brokered model ids", () => {
    expect(isClaudeFamilyModel("anthropic/claude-sonnet-4-6")).toBe(true);
    expect(isClaudeFamilyModel("openrouter/anthropic/claude-opus-4.1")).toBe(true);
    expect(isClaudeFamilyModel("amazon-bedrock/us.anthropic.claude-4")).toBe(true);
  });

  it("does not classify missing or non-Claude models as Claude", () => {
    expect(isClaudeFamilyModel("openai/gpt-5.4")).toBe(false);
    expect(isClaudeFamilyModel("google/gemini-3-pro")).toBe(false);
    expect(isClaudeFamilyModel(null)).toBe(false);
  });
});

describe("resolvesInHarness", () => {
  const entry = (name: string, kind: SlashCommand["kind"]): SlashCommand => ({
    name,
    description: "",
    source: "user",
    kind,
  });

  it("offers a skill to codex, which is handed the .claude/skills roots", () => {
    expect(resolvesInHarness(entry("create_plan", "skill"), "codex")).toBe(true);
  });

  it("hides a prompt command from codex, which has nothing to expand it", () => {
    expect(resolvesInHarness(entry("git:sync", "command"), "codex")).toBe(false);
  });

  it("leaves every other harness's offering untouched", () => {
    for (const harness of ["claude", "opencode"]) {
      expect(resolvesInHarness(entry("git:sync", "command"), harness)).toBe(true);
      expect(resolvesInHarness(entry("create_plan", "skill"), harness)).toBe(true);
    }
  });
});
