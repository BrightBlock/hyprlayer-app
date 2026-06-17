import { describe, expect, it } from "vitest";

import {
  baseName,
  docFromTool,
  docTitle,
  editDiffFromTool,
  formatCost,
  formatElapsed,
  formatTokens,
  parseToolInput,
  prettyToolInput,
  resultPreview,
  toolFilePath,
  toolSummary,
} from "./toolDisplay";

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

  it("surfaces the question text for AskUserQuestion", () => {
    expect(toolSummary("AskUserQuestion", '{"questions":[{"question":"Tabs or spaces?"}]}')).toBe(
      "Tabs or spaces?",
    );
    expect(
      toolSummary("AskUserQuestion", '{"questions":[{"question":"A?"},{"question":"B?"}]}'),
    ).toBe("A? (+1 more)");
  });

  it("falls back to the first string arg for unknown tools", () => {
    expect(toolSummary("Mystery", '{"n":1,"label":"hello"}')).toBe("hello");
    expect(toolSummary("Mystery", '{"n":1}')).toBe("");
  });

  it("is empty when input does not parse", () => {
    expect(toolSummary("Bash", "garbage")).toBe("");
  });
});

describe("toolFilePath", () => {
  it("returns the path for file-targeting tools", () => {
    expect(toolFilePath("Read", '{"file_path":"/a/b.ts"}')).toBe("/a/b.ts");
    expect(toolFilePath("Edit", '{"file_path":"/a/b.ts","old_string":"x"}')).toBe("/a/b.ts");
    expect(toolFilePath("Write", '{"path":"/a/c.md"}')).toBe("/a/c.md");
    expect(toolFilePath("NotebookEdit", '{"notebook_path":"/a/n.ipynb"}')).toBe("/a/n.ipynb");
  });

  it("returns null for non-file tools and unparseable input", () => {
    expect(toolFilePath("Bash", '{"command":"ls"}')).toBeNull();
    expect(toolFilePath("Grep", '{"pattern":"foo","path":"src"}')).toBeNull();
    expect(toolFilePath("Read", "garbage")).toBeNull();
    expect(toolFilePath("Read", "{}")).toBeNull();
  });
});

describe("editDiffFromTool", () => {
  it("derives a diff from an Edit's old/new strings", () => {
    const diff = editDiffFromTool(
      "Edit",
      JSON.stringify({ file_path: "/a/b.ts", old_string: "a\nold\nc", new_string: "a\nnew\nc" }),
    );
    expect(diff).toEqual({
      lines: [
        { kind: "context", text: " a" },
        { kind: "del", text: "-old" },
        { kind: "add", text: "+new" },
        { kind: "context", text: " c" },
      ],
      additions: 1,
      deletions: 1,
    });
  });

  it("stacks a MultiEdit's edits with a hunk separator", () => {
    const diff = editDiffFromTool(
      "MultiEdit",
      JSON.stringify({
        file_path: "/a/b.ts",
        edits: [
          { old_string: "x", new_string: "y" },
          { old_string: "p", new_string: "q" },
        ],
      }),
    );
    expect(diff?.additions).toBe(2);
    expect(diff?.deletions).toBe(2);
    expect(diff?.lines.map((l) => l.kind)).toEqual([
      "del",
      "add",
      "hunk",
      "del",
      "add",
    ]);
  });

  it("returns null for non-edit tools and unparseable input", () => {
    expect(editDiffFromTool("Read", '{"file_path":"/a/b.ts"}')).toBeNull();
    expect(editDiffFromTool("Edit", "garbage")).toBeNull();
    expect(editDiffFromTool("MultiEdit", '{"file_path":"/a/b.ts"}')).toBeNull();
  });
});

describe("baseName", () => {
  it("returns the final path segment", () => {
    expect(baseName("/Users/jt/repo/src/main.rs")).toBe("main.rs");
    expect(baseName("file.ts")).toBe("file.ts");
    expect(baseName("/a/b/")).toBe("b");
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

describe("resultPreview", () => {
  it("strips cat -n line numbers and shows the first meaningful line", () => {
    expect(resultPreview("  12\tuse anyhow::Result;", false)).toBe("use anyhow::Result;");
    expect(resultPreview("\n\n   first real line\nsecond", false)).toBe("first real line");
  });

  it("falls back when there is no output", () => {
    expect(resultPreview("", false)).toBe("(no output)");
    expect(resultPreview("   \n  ", true)).toBe("(error)");
  });

  it("leaves ordinary content untouched", () => {
    expect(resultPreview("Done (12 tool uses)", false)).toBe("Done (12 tool uses)");
  });
});

describe("docFromTool", () => {
  it("recognises research and plan docs written by the agent", () => {
    expect(
      docFromTool("Write", '{"file_path":"/Users/jt/thoughts/repos/x/shared/research/2026-06-15-auth.md"}'),
    ).toEqual({ path: "/Users/jt/thoughts/repos/x/shared/research/2026-06-15-auth.md", kind: "research" });
    expect(docFromTool("Edit", '{"file_path":"/t/shared/plans/big-refactor.md"}')).toEqual({
      path: "/t/shared/plans/big-refactor.md",
      kind: "plan",
    });
  });

  it("ignores non-doc writes and non-write tools", () => {
    expect(docFromTool("Write", '{"file_path":"/repo/src/main.rs"}')).toBeNull();
    expect(docFromTool("Write", '{"file_path":"/t/shared/research/notes.txt"}')).toBeNull();
    expect(docFromTool("Read", '{"file_path":"/t/shared/research/x.md"}')).toBeNull();
    expect(docFromTool("Bash", '{"command":"ls"}')).toBeNull();
  });
});

describe("docTitle", () => {
  it("strips the date prefix and extension, humanizing the slug", () => {
    expect(docTitle("/t/research/2026-06-15-libhyprlayer-crate-extraction.md")).toBe(
      "libhyprlayer crate extraction",
    );
    expect(docTitle("/t/plans/big_refactor.md")).toBe("big refactor");
  });
});

describe("formatTokens", () => {
  it("scales to k and M", () => {
    expect(formatTokens(980)).toBe("980");
    expect(formatTokens(24130)).toBe("24.1k");
    expect(formatTokens(269_500)).toBe("269.5k");
    expect(formatTokens(1_000_000)).toBe("1.0M");
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

describe("formatElapsed", () => {
  it("shows tenths of a second under a minute", () => {
    expect(formatElapsed(0)).toBe("0.0s");
    expect(formatElapsed(19_300)).toBe("19.3s");
    expect(formatElapsed(59_900)).toBe("59.9s");
  });

  it("switches to minutes + zero-padded seconds at a minute", () => {
    expect(formatElapsed(60_000)).toBe("1m 00s");
    expect(formatElapsed(83_000)).toBe("1m 23s");
    expect(formatElapsed(605_000)).toBe("10m 05s");
  });

  it("switches to hours + minutes past an hour", () => {
    expect(formatElapsed(3_600_000)).toBe("1h 00m");
    expect(formatElapsed(3_840_000)).toBe("1h 04m");
  });

  it("clamps negative inputs to zero", () => {
    expect(formatElapsed(-500)).toBe("0.0s");
  });
});
