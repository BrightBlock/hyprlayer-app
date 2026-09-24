import { describe, expect, it } from "vitest";

import { planDocForSession } from "./planLink";
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

const write = (path: string): TranscriptItem => ({
  kind: "tool",
  id: "i2",
  toolUseId: "tu",
  name: "Write",
  input: JSON.stringify({ file_path: path, content: "" }),
  result: null,
});

describe("planDocForSession", () => {
  it("is null without a user turn or plan reference", () => {
    expect(planDocForSession([], "implement", {})).toBeNull();
    expect(planDocForSession([user("hello")], "implement", {})).toBeNull();
  });

  it("reads the first plan from the Relevant documents list", () => {
    const text = `/implement_plan\n\nRelevant documents:\n- /t/research/r.md\n- ${PLAN}\n- /t/plans/other.md`;
    expect(planDocForSession([user(text)], "implement", {})).toBe(PLAN);
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
    expect(planDocForSession([live], "implement", {})).toBe(PLAN);
  });

  it("falls back to the /implement_plan argument", () => {
    expect(planDocForSession([user(`/implement_plan ${PLAN}`)], "implement", {})).toBe(PLAN);
  });

  it("ignores later user turns", () => {
    expect(planDocForSession([user("go"), user(`/implement_plan ${PLAN}`)], "implement", {})).toBeNull();
  });

  it("prefers the plan a plan-stage session wrote", () => {
    const items = [user(`/create_plan x\n\nRelevant documents:\n- /t/plans/old.md`), write(PLAN)];
    expect(planDocForSession(items, "plan", {})).toBe(PLAN);
    expect(planDocForSession(items, "implement", {})).toBe("/t/plans/old.md");
  });
});
