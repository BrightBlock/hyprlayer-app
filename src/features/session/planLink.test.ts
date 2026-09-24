import { describe, expect, it } from "vitest";

import { planKeyForSession } from "./planPresentation";
import type { TranscriptItem } from "./transcript";

const PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";

const user = (text: string): TranscriptItem => ({
  kind: "user",
  id: "i1",
  text,
  stage: null,
  attachments: [],
  createdAt: null,
  promptName: null,
});

const write = (
  path: string,
  result: { content: string; isError: boolean } | null = { content: "ok", isError: false },
): TranscriptItem => ({
  kind: "tool",
  id: "i2",
  toolUseId: "tu",
  name: "Write",
  input: JSON.stringify({ file_path: path, content: "" }),
  result,
});

describe("planKeyForSession", () => {
  it("is null without a user turn or plan reference", () => {
    expect(planKeyForSession([], "implement", {})).toBeNull();
    expect(planKeyForSession([user("hello")], "implement", {})).toBeNull();
  });

  it("reads the first plan from the Relevant documents list", () => {
    const text = `/implement_plan\n\nRelevant documents:\n- /t/research/r.md\n- ${PLAN}\n- /t/plans/other.md`;
    expect(planKeyForSession([user(text)], "implement", {})).toBe(PLAN);
  });

  it("reads the attachments of a live first turn", () => {
    const live: TranscriptItem = {
      kind: "user",
      id: "i1",
      text: "/implement_plan",
      stage: "implement",
      attachments: [{ path: PLAN, name: "og", kind: "file" }],
      createdAt: null,
      promptName: null,
    };
    expect(planKeyForSession([live], "implement", {})).toBe(PLAN);
  });

  it("falls back to the /implement_plan argument", () => {
    expect(planKeyForSession([user(`/implement_plan ${PLAN}`)], "implement", {})).toBe(PLAN);
  });

  it("ignores later user turns", () => {
    expect(planKeyForSession([user("go"), user(`/implement_plan ${PLAN}`)], "implement", {})).toBeNull();
  });

  it("prefers the plan a plan-stage session wrote", () => {
    const items = [user(`/create_plan x\n\nRelevant documents:\n- /t/plans/old.md`), write(PLAN)];
    expect(planKeyForSession(items, "plan", {})).toBe(PLAN);
    expect(planKeyForSession(items, "implement", {})).toBe("/t/plans/old.md");
  });

  it("ignores a plan write that has not finished or was denied", () => {
    const first = user("/create_plan x");
    expect(planKeyForSession([first, write(PLAN, null)], "plan", {})).toBeNull();
    expect(planKeyForSession([first, write(PLAN, { content: "denied", isError: true })], "plan", {})).toBeNull();
  });
});
