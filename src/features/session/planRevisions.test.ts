import { describe, expect, it } from "vitest";

import type { TranscriptItem } from "./transcript";

import { planRevisions } from "./planRevisions";

const PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";
const OTHER = "/t/repos/app/shared/plans/2026-09-24-other.md";

let n = 0;
const user = (): TranscriptItem => ({
  kind: "user",
  id: `u${n++}`,
  text: "go",
  stage: null,
  attachments: [],
  createdAt: null,
  promptName: null,
});

const tool = (
  toolUseId: string,
  name: string,
  input: Record<string, unknown>,
  result: { content: string; isError: boolean } | null = { content: "ok", isError: false },
  parentId: string | null = null,
): TranscriptItem => ({
  kind: "tool",
  id: `i-${toolUseId}`,
  toolUseId,
  name,
  input: JSON.stringify(input),
  result,
  parentId,
});

const write = (id: string, path: string, result?: { content: string; isError: boolean } | null) =>
  tool(id, "Write", { file_path: path, content: "" }, result);
const edit = (id: string, path: string) => tool(id, "Edit", { file_path: path, old_string: "a", new_string: "b" });

describe("planRevisions", () => {
  it("is empty when no plan was written", () => {
    expect(planRevisions([user(), tool("t1", "Read", { file_path: PLAN })], {}).size).toBe(0);
    expect(planRevisions([user(), write("t1", "/repo/src/app.md")], {}).size).toBe(0);
  });

  it("makes one card per turn per plan, at the last write", () => {
    const revs = planRevisions([user(), write("t1", PLAN), edit("t2", PLAN), edit("t3", PLAN)], {});
    expect([...revs.keys()]).toEqual(["t3"]);
    expect(revs.get("t3")).toEqual({ planKey: PLAN, text: null, superseded: false, latest: true });
  });

  it("marks every revision but the latest superseded", () => {
    const revs = planRevisions([user(), write("t1", PLAN), user(), edit("t2", PLAN), user(), edit("t3", PLAN)], {});
    expect([...revs.entries()]).toEqual([
      ["t1", { planKey: PLAN, text: null, superseded: true, latest: false }],
      ["t2", { planKey: PLAN, text: null, superseded: true, latest: false }],
      ["t3", { planKey: PLAN, text: null, superseded: false, latest: true }],
    ]);
  });

  it("skips in-flight and failed writes", () => {
    const revs = planRevisions(
      [user(), write("t1", PLAN), user(), write("t2", PLAN, null), user(), write("t3", PLAN, { content: "denied", isError: true })],
      {},
    );
    expect([...revs.entries()]).toEqual([["t1", { planKey: PLAN, text: null, superseded: false, latest: true }]]);
  });

  it("tracks two plans in one session independently", () => {
    const revs = planRevisions([user(), write("t1", PLAN), write("t2", OTHER), user(), edit("t3", PLAN)], {});
    expect([...revs.entries()]).toEqual([
      ["t1", { planKey: PLAN, text: null, superseded: true, latest: false }],
      ["t2", { planKey: OTHER, text: null, superseded: false, latest: false }],
      ["t3", { planKey: PLAN, text: null, superseded: false, latest: true }],
    ]);
  });

  it("resolves a shell redirect through the written-docs index", () => {
    const bash = tool("t1", "Bash", { command: "cat > 2026-09-24-og.md <<'EOF'\n# x\nEOF" });
    const revs = planRevisions([user(), bash], { "2026-09-24-og.md": PLAN });
    expect(revs.get("t1")).toEqual({ planKey: PLAN, text: null, superseded: false, latest: true });
  });

  it("ignores writes inside a subagent", () => {
    const nested = tool("t1", "Write", { file_path: PLAN, content: "" }, undefined, "agent-1");
    expect(planRevisions([user(), nested], {}).size).toBe(0);
  });

  it("puts one card on the ExitPlanMode that presents a plan written in the same turn", () => {
    const exit = tool("x1", "ExitPlanMode", { plan: "# P", planFilePath: PLAN }, null);
    const revs = planRevisions([user(), write("t1", PLAN), exit], {});
    expect([...revs.entries()]).toEqual([["x1", { planKey: PLAN, text: null, superseded: false, latest: true }]]);
  });

  it("lets a later inline plan supersede an earlier one and carries the newest text", () => {
    const first = tool("c1", "ProposedPlan", { plan: "# One" });
    const second = tool("c2", "ProposedPlan", { plan: "# Two" });
    const revs = planRevisions([user(), first, user(), second], {});
    expect([...revs.entries()]).toEqual([
      ["c1", { planKey: "inline", text: "# Two", superseded: true, latest: false }],
      ["c2", { planKey: "inline", text: "# Two", superseded: false, latest: true }],
    ]);
  });

  it("tracks a doc plan and an inline plan in one session", () => {
    const inline = tool("c1", "ProposedPlan", { plan: "# Draft" });
    const revs = planRevisions([user(), inline, user(), write("t1", PLAN)], {});
    expect([...revs.entries()].map(([id, r]) => [id, r.planKey, r.superseded, r.latest])).toEqual([
      ["c1", "inline", false, false],
      ["t1", PLAN, false, true],
    ]);
  });
});
