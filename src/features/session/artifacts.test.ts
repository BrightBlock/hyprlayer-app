import { describe, expect, it } from "vitest";

import { artifactFromTool, artifactsFromItems } from "./artifacts";
import type { TranscriptItem } from "./transcript";

// ─── helpers ────────────────────────────────────────────────────────────────

function makeTool(
  name: string,
  input: string,
  toolUseId: string,
  result: { content: string; isError: boolean } | null = null,
): Extract<TranscriptItem, { kind: "tool" }> {
  return { kind: "tool", id: `id-${toolUseId}`, toolUseId, name, input, result };
}

const ok = (content: string) => ({ content, isError: false });
const err = (content: string) => ({ content, isError: true });

const REAL_RESULT = ok(
  "Published /home/user/.claude/scratchpad/design/out/linux-and-codex-rollout.html at " +
    "https://claude.ai/code/artifact/05f7c471-efe9-4a18-ac8e-66f193a330fd\n\n" +
    "Live subscription: skipped — only an interactive or SDK main-loop session holds the watch.",
);

function artifactTool(
  input: Record<string, unknown>,
  result: { content: string; isError: boolean } | null = REAL_RESULT,
  toolUseId = "t1",
): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("Artifact", JSON.stringify(input), toolUseId, result);
}

// ─── artifactFromTool ───────────────────────────────────────────────────────

describe("artifactFromTool", () => {
  it("parses the captured real-world publish", () => {
    const artifact = artifactFromTool(
      "Artifact",
      JSON.stringify({
        file_path: "/home/user/.claude/scratchpad/design/out/linux-and-codex-rollout.html",
        description: "Three hero install-row options for adding Linux to hyprlayer.dev",
        favicon: "🐧📦",
        capabilities: { artifact: {}, downloads: {} },
      }),
      REAL_RESULT,
    );
    expect(artifact).toEqual({
      url: "https://claude.ai/code/artifact/05f7c471-efe9-4a18-ac8e-66f193a330fd",
      title: "linux and codex rollout",
      description: "Three hero install-row options for adding Linux to hyprlayer.dev",
      filePath: "/home/user/.claude/scratchpad/design/out/linux-and-codex-rollout.html",
    });
  });

  it("prefers an explicit title input over the filename", () => {
    const artifact = artifactFromTool(
      "Artifact",
      JSON.stringify({ file_path: "/tmp/out.html", title: "Perf Dashboard" }),
      REAL_RESULT,
    );
    expect(artifact?.title).toBe("Perf Dashboard");
  });

  it.each([
    ["/tmp/scratch/design.dc.html", "design"],
    ["/tmp/scratch/notes-on-things.md", "notes on things"],
    ["/tmp/scratch/README", "README"],
  ])("derives a sane title from %s", (filePath, expected) => {
    const artifact = artifactFromTool("Artifact", JSON.stringify({ file_path: filePath }), REAL_RESULT);
    expect(artifact?.title).toBe(expected);
  });

  it.each(["list", "comments", "reply", "resolve"])(
    "yields null for action %s even when the result contains a claude.ai URL",
    (action) => {
      const artifact = artifactFromTool(
        "Artifact",
        JSON.stringify({ action, url: "https://claude.ai/code/artifact/abc" }),
        ok("Published https://claude.ai/code/artifact/abc"),
      );
      expect(artifact).toBeNull();
    },
  );

  it("treats an omitted action, and an explicit \"publish\", as a publish", () => {
    expect(
      artifactFromTool("Artifact", JSON.stringify({ file_path: "/tmp/a.html" }), REAL_RESULT),
    ).not.toBeNull();
    expect(
      artifactFromTool(
        "Artifact",
        JSON.stringify({ action: "publish", file_path: "/tmp/a.html" }),
        REAL_RESULT,
      ),
    ).not.toBeNull();
  });

  it("yields null when the result hasn't landed yet", () => {
    expect(artifactFromTool("Artifact", JSON.stringify({ file_path: "/tmp/a.html" }), null)).toBeNull();
  });

  it("yields null when the publish errored", () => {
    expect(
      artifactFromTool(
        "Artifact",
        JSON.stringify({ file_path: "/tmp/a.html" }),
        err("publish failed: network error"),
      ),
    ).toBeNull();
  });

  it("yields null when the result's first line isn't a Published … at <url> line", () => {
    expect(
      artifactFromTool("Artifact", JSON.stringify({ file_path: "/tmp/a.html" }), ok("something else entirely")),
    ).toBeNull();
  });

  it("yields null for a non-Artifact tool", () => {
    expect(artifactFromTool("Write", JSON.stringify({ file_path: "/tmp/a.html" }), REAL_RESULT)).toBeNull();
  });
});

// ─── artifactsFromItems ─────────────────────────────────────────────────────

describe("artifactsFromItems", () => {
  it("returns empty for an empty transcript", () => {
    expect(artifactsFromItems([])).toEqual([]);
  });

  it("collapses two publishes of the same URL into one entry, keeping first position and latest title", () => {
    const firstResult = ok("Published /tmp/a.html at https://claude.ai/code/artifact/same-url");
    const secondResult = ok("Published /tmp/a-renamed.html at https://claude.ai/code/artifact/same-url");
    const items: TranscriptItem[] = [
      artifactTool({ file_path: "/tmp/a.html", description: "first" }, firstResult, "t1"),
      artifactTool({ file_path: "/tmp/other.html" }, ok("Published /tmp/other.html at https://claude.ai/code/artifact/other"), "t2"),
      artifactTool({ file_path: "/tmp/a-renamed.html", description: "second" }, secondResult, "t3"),
    ];
    const artifacts = artifactsFromItems(items);
    expect(artifacts.map((a) => a.url)).toEqual([
      "https://claude.ai/code/artifact/same-url",
      "https://claude.ai/code/artifact/other",
    ]);
    expect(artifacts[0].title).toBe("a renamed");
    expect(artifacts[0].description).toBe("second");
  });

  it("keeps two publishes of different URLs, in transcript order", () => {
    const items: TranscriptItem[] = [
      artifactTool(
        { file_path: "/tmp/b.html" },
        ok("Published /tmp/b.html at https://claude.ai/code/artifact/b"),
        "t1",
      ),
      artifactTool(
        { file_path: "/tmp/a.html" },
        ok("Published /tmp/a.html at https://claude.ai/code/artifact/a"),
        "t2",
      ),
    ];
    expect(artifactsFromItems(items).map((a) => a.url)).toEqual([
      "https://claude.ai/code/artifact/b",
      "https://claude.ai/code/artifact/a",
    ]);
  });

  it("ignores non-tool and unrelated tool items", () => {
    const items: TranscriptItem[] = [
      { kind: "user", id: "u1", text: "hi", stage: null, attachments: [], createdAt: null, promptName: null },
      { kind: "assistant", id: "a1", text: "ok", streaming: false },
      makeTool("Bash", JSON.stringify({ command: "ls" }), "t1", ok("done")),
    ];
    expect(artifactsFromItems(items)).toEqual([]);
  });
});
