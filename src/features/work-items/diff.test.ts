import { describe, expect, it } from "vitest";

import { classifyPatchLine, countPatchStats, parsePatchLines } from "./diff";

const SAMPLE_PATCH = [
  "diff --git a/src/app.ts b/src/app.ts",
  "index 1111111..2222222 100644",
  "--- a/src/app.ts",
  "+++ b/src/app.ts",
  "@@ -1,4 +1,5 @@",
  " import { boot } from './boot';",
  "-const mode = 'old';",
  "+const mode = 'new';",
  "+const extra = true;",
  " boot(mode);",
  "\\ No newline at end of file",
  "",
].join("\n");

describe("classifyPatchLine", () => {
  it("classifies content lines", () => {
    expect(classifyPatchLine("+added")).toBe("add");
    expect(classifyPatchLine("-removed")).toBe("del");
    expect(classifyPatchLine(" context")).toBe("context");
    expect(classifyPatchLine("")).toBe("context");
  });

  it("does not count file headers as content", () => {
    expect(classifyPatchLine("+++ b/file.ts")).toBe("meta");
    expect(classifyPatchLine("--- a/file.ts")).toBe("meta");
  });

  it("classifies hunk and git metadata", () => {
    expect(classifyPatchLine("@@ -1,4 +1,5 @@")).toBe("hunk");
    expect(classifyPatchLine("diff --git a/x b/x")).toBe("meta");
    expect(classifyPatchLine("index 1111111..2222222 100644")).toBe("meta");
    expect(classifyPatchLine("new file mode 100644")).toBe("meta");
    expect(classifyPatchLine("deleted file mode 100644")).toBe("meta");
    expect(classifyPatchLine("rename from a.ts")).toBe("meta");
    expect(classifyPatchLine("rename to b.ts")).toBe("meta");
    expect(classifyPatchLine("Binary files a/x and b/x differ")).toBe("meta");
    expect(classifyPatchLine("\\ No newline at end of file")).toBe("meta");
  });

  it("treats +++/--- lookalikes inside content as content", () => {
    // A removed line that begins with a single '-' then dashes is still a del.
    expect(classifyPatchLine("-- comment")).toBe("del");
    expect(classifyPatchLine("+- mixed")).toBe("add");
  });
});

describe("countPatchStats", () => {
  it("counts only real content lines", () => {
    expect(countPatchStats(SAMPLE_PATCH)).toEqual({ additions: 2, deletions: 1 });
  });

  it("handles empty patches", () => {
    expect(countPatchStats("")).toEqual({ additions: 0, deletions: 0 });
  });
});

describe("parsePatchLines", () => {
  it("classifies every line and drops the trailing blank", () => {
    const lines = parsePatchLines(SAMPLE_PATCH);
    expect(lines).toHaveLength(11);
    expect(lines[0]).toEqual({ kind: "meta", text: "diff --git a/src/app.ts b/src/app.ts" });
    expect(lines[4].kind).toBe("hunk");
    expect(lines[5].kind).toBe("context");
    expect(lines[6].kind).toBe("del");
    expect(lines[7].kind).toBe("add");
    expect(lines[10].kind).toBe("meta");
  });

  it("keeps interior blank lines as context", () => {
    const lines = parsePatchLines("+a\n\n-b\n");
    expect(lines.map((l) => l.kind)).toEqual(["add", "context", "del"]);
  });
});
