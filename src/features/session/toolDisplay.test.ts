import { describe, expect, it } from "vitest";

import { formatCost, parseToolInput, prettyToolInput, toolSummary } from "./toolDisplay";

describe("parseToolInput", () => {
  it("parses a JSON object", () => {
    expect(parseToolInput('{"a":1}')).toEqual({ a: 1 });
  });

  it("rejects non-objects and junk", () => {
    expect(parseToolInput("[]")).toBeNull();
    expect(parseToolInput('"hi"')).toBeNull();
    expect(parseToolInput("not json")).toBeNull();
    expect(parseToolInput("")).toBeNull();
    expect(parseToolInput("null")).toBeNull();
  });
});

describe("toolSummary", () => {
  it("surfaces the bash command, collapsing to one line", () => {
    expect(toolSummary("Bash", '{"command":"git status"}')).toBe("git status");
    expect(toolSummary("Bash", '{"command":"line one\\nline two"}')).toBe("line one …");
  });

  it("surfaces file paths for file tools", () => {
    expect(toolSummary("Read", '{"file_path":"/a/b.ts"}')).toBe("/a/b.ts");
    expect(toolSummary("Edit", '{"file_path":"/a/b.ts","old_string":"x"}')).toBe("/a/b.ts");
    expect(toolSummary("Write", '{"path":"/a/c.md"}')).toBe("/a/c.md");
  });

  it("surfaces search patterns with optional scope", () => {
    expect(toolSummary("Grep", '{"pattern":"foo"}')).toBe("foo");
    expect(toolSummary("Grep", '{"pattern":"foo","path":"src"}')).toBe("foo  in src");
    expect(toolSummary("Glob", '{"pattern":"**/*.ts"}')).toBe("**/*.ts");
  });

  it("summarises task / web / todo tools", () => {
    expect(toolSummary("Task", '{"description":"find bugs","subagent_type":"x"}')).toBe("find bugs");
    expect(toolSummary("WebSearch", '{"query":"tauri channel"}')).toBe("tauri channel");
    expect(toolSummary("TodoWrite", '{"todos":[{},{},{}]}')).toBe("3 items");
  });

  it("falls back to the first string arg for unknown tools", () => {
    expect(toolSummary("Mystery", '{"n":1,"label":"hello"}')).toBe("hello");
    expect(toolSummary("Mystery", '{"n":1}')).toBe("");
  });

  it("is empty when input does not parse", () => {
    expect(toolSummary("Bash", "garbage")).toBe("");
  });
});

describe("prettyToolInput", () => {
  it("pretty-prints valid JSON", () => {
    expect(prettyToolInput('{"a":1}')).toBe('{\n  "a": 1\n}');
  });

  it("returns the raw string for unparseable input", () => {
    expect(prettyToolInput("not json")).toBe("not json");
    expect(prettyToolInput("")).toBe("");
  });
});

describe("formatCost", () => {
  it("uses 4dp under a dollar and 2dp above", () => {
    expect(formatCost(0.0042)).toBe("$0.0042");
    expect(formatCost(1.5)).toBe("$1.50");
    expect(formatCost(0)).toBe("$0.0000");
  });

  it("is null for missing / non-finite values", () => {
    expect(formatCost(null)).toBeNull();
    expect(formatCost(undefined)).toBeNull();
    expect(formatCost(Number.NaN)).toBeNull();
  });
});
