import { describe, expect, it } from "vitest";

import { reviewAnchor } from "./reviewAnchor";
import type { TranscriptItem } from "./transcript";

const WT = "/w/repo";

const user = (id: string): TranscriptItem =>
  ({
    kind: "user",
    id,
    text: id,
    stage: null,
    attachments: [],
    createdAt: null,
    promptName: null,
    clientMessageId: null,
  }) as TranscriptItem;

const tool = (id: string, name: string, input: Record<string, unknown>, parentId: string | null = null): TranscriptItem =>
  ({
    kind: "tool",
    id,
    toolUseId: id,
    name,
    input: JSON.stringify(input),
    result: null,
    parentId,
  }) as TranscriptItem;

const edit = (id: string, file: string, parentId: string | null = null) =>
  tool(id, "Edit", { file_path: file, old_string: "a", new_string: "b" }, parentId);
const read = (id: string, file: string) => tool(id, "Read", { file_path: file });

describe("reviewAnchor", () => {
  it("is null with no turn that edited the worktree", () => {
    expect(reviewAnchor([user("u1"), read("r1", `${WT}/a.ts`), user("u2")], false, WT)).toBeNull();
  });

  it("is null when the newest finished turn made the edits, so the card closes the transcript", () => {
    expect(reviewAnchor([user("u1"), read("r1", `${WT}/a.ts`), user("u2"), edit("e1", `${WT}/a.ts`)], false, WT)).toBeNull();
  });

  it("stays before the chat that followed the last edit", () => {
    const items = [user("u1"), edit("e1", `${WT}/a.ts`), user("u2"), read("r1", `${WT}/a.ts`), user("u3")];
    expect(reviewAnchor(items, false, WT)).toBe("u2");
  });

  it("stays above a running turn, even one that is editing", () => {
    const items = [user("u1"), edit("e1", `${WT}/a.ts`), user("u2"), edit("e2", `${WT}/b.ts`)];
    expect(reviewAnchor(items, true, WT)).toBe("u2");
  });

  it("stays above a running turn when no turn's edits were seen", () => {
    expect(reviewAnchor([user("u1"), read("r1", `${WT}/a.ts`), user("u2")], true, WT)).toBe("u2");
  });

  it("ignores edits to thoughts and to files outside the worktree", () => {
    const items = [
      user("u1"),
      edit("e1", `${WT}/a.ts`),
      user("u2"),
      edit("e2", `${WT}/thoughts/shared/plans/p.md`),
      edit("e3", "/home/jt/thoughts/plans/p.md"),
      edit("e4", `${WT}-other/a.ts`),
    ];
    expect(reviewAnchor(items, false, WT)).toBe("u2");
  });

  it("counts a subagent's edit and a relative path", () => {
    const viaSubagent = [user("u1"), tool("t1", "Task", {}), edit("e1", `${WT}/a.ts`, "t1"), user("u2"), edit("e2", "thoughts/x.md")];
    expect(reviewAnchor(viaSubagent, false, WT)).toBe("u2");
    expect(reviewAnchor([user("u1"), read("r1", "a.ts"), user("u2"), tool("w1", "Write", { file_path: "src/a.ts" })], false, WT)).toBeNull();
  });
});
