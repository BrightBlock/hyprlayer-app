import { describe, expect, it } from "vitest";

import { detectState, foot, stripAnsi, type AgentState } from "./engineState";

describe("stripAnsi", () => {
  it("removes CSI color sequences", () => {
    expect(stripAnsi("\x1b[31mred\x1b[0m")).toBe("red");
  });
  it("folds carriage returns to newlines (spinner redraws)", () => {
    expect(stripAnsi("frame1\rframe2")).toBe("frame1\nframe2");
  });
});

describe("foot", () => {
  it("returns the last non-empty lines", () => {
    expect(foot("a\nb\nc\n\n\n", 2)).toBe("b\nc");
  });
});

describe("detectState", () => {
  const blocked = (engineId: string, tail: string) =>
    detectState(engineId, tail, true, "working");

  it("reports working while output flows and nothing blocks", () => {
    expect(detectState("claude-code", "✶ Cogitating… (esc to interrupt)", false, "idle")).toBe(
      "working",
    );
  });

  it("blocks on Claude Code's permission prompt even mid-stream", () => {
    expect(detectState("claude-code", "Do you want to proceed?", false, "working")).toBe("blocked");
    expect(blocked("claude-code", "❯ 1. Yes\n  2. No")).toBe("blocked");
  });

  it("blocks on Codex approval prompts", () => {
    expect(blocked("codex", "Approve shell command? rm -rf build")).toBe("blocked");
    expect(blocked("codex", "Run this command?")).toBe("blocked");
  });

  it("blocks on Gemini approval prompts", () => {
    expect(blocked("gemini", "Apply this change? (Y/n)")).toBe("blocked");
  });

  it("blocks on a generic y/n confirmation for any engine", () => {
    expect(blocked("custom-cli", "Overwrite file? (y/n)")).toBe("blocked");
  });

  it("blocks when quiet and the last line is a bare question", () => {
    expect(detectState("custom-cli", "What should I name it?", true, "working")).toBe("blocked");
  });

  it("does NOT block on a ready shell prompt", () => {
    expect(detectState("custom-cli", "user@host project %", true, "idle")).toBe("idle");
  });

  it("transitions working → done when output goes quiet", () => {
    expect(detectState("claude-code", "● Done. 1 file changed\n❯ ", true, "working")).toBe("done");
  });

  it("stays idle on a fresh quiet spawn, sticks at done once finished", () => {
    expect(detectState("claude-code", "$ ", true, "idle")).toBe("idle");
    expect(detectState("claude-code", "$ ", true, "done")).toBe("done");
  });

  it("ignores a y/n that isn't on the live prompt (last 2 lines)", () => {
    const tail = "Continue? (y/n) y\nRunning task…\n● Done\n❯ ";
    expect(detectState("custom-cli", tail, true, "working")).toBe("done");
  });

  it("does not block on a long done-message that merely ends in a question", () => {
    const tail =
      "I've finished. Want me to also update the docs and the changelog and then open a PR?";
    expect(detectState("claude-code", tail, true, "working")).toBe("done");
  });

  it("never matches blocked patterns sitting in old scrollback", () => {
    const tail = `Overwrite? (y/n) y\n${"output line\n".repeat(20)}❯ `;
    const next: AgentState = detectState("claude-code", tail, true, "working");
    expect(next).toBe("done");
  });
});
