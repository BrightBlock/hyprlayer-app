import { describe, expect, it } from "vitest";

import { detectTrigger, highlightSegments } from "./ComposerAutocomplete";

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
});
