import { describe, expect, it } from "vitest";

import { INLINE_PLAN_KEY, planKeyForSession, planPresentations, presentationFor } from "./planPresentation";
import type { TranscriptItem } from "./transcript";

const CLAUDE_PLAN = "/Users/jt/.claude/plans/staged-sleeping-octopus.md";
const THOUGHTS_PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";

let n = 0;
const user = (text = "go"): TranscriptItem => ({
  kind: "user",
  id: `u${n++}`,
  text,
  stage: null,
  attachments: [],
  createdAt: null,
  promptName: null,
});
const tool = (
  toolUseId: string,
  name: string,
  input: object,
  result: { content: string; isError: boolean } | null = { content: "ok", isError: false },
): TranscriptItem => ({ kind: "tool", id: `i-${toolUseId}`, toolUseId, name, input: JSON.stringify(input), result });
const permission = (requestId: string, toolName: string, decision: "allow" | "deny" | null = null): TranscriptItem => ({
  kind: "permission",
  id: `p-${requestId}`,
  requestId,
  toolName,
  input: "{}",
  decision,
});
const write = (id: string, path: string) => tool(id, "Write", { file_path: path, content: "" });
const exitPlan = (id: string, input: object, result: { content: string; isError: boolean } | null = null) =>
  tool(id, "ExitPlanMode", input, result);

describe("planPresentations", () => {
  it("reads Claude's ExitPlanMode with its plan file, pending on the permission prompt", () => {
    const items = [user(), write("w1", CLAUDE_PLAN), exitPlan("x1", { plan: "# P", planFilePath: CLAUDE_PLAN }), permission("r1", "ExitPlanMode")];
    expect(planPresentations(items, {})).toEqual([
      { toolUseId: "x1", tool: "ExitPlanMode", key: CLAUDE_PLAN, text: "# P", state: "pending", requestId: "r1" },
    ]);
  });

  it("falls back to the plan text when ExitPlanMode names no file and none was written", () => {
    const [p] = planPresentations([user(), exitPlan("x1", { plan: "# Inline" })], {});
    expect(p).toMatchObject({ key: INLINE_PLAN_KEY, text: "# Inline" });
  });

  it("uses the plan file written earlier when ExitPlanMode names no file", () => {
    const [p] = planPresentations([user(), write("w1", CLAUDE_PLAN), exitPlan("x1", { plan: "# P" })], {});
    expect(p.key).toBe(CLAUDE_PLAN);
  });

  it("marks an allowed ExitPlanMode approved and a denied one rejected", () => {
    const allowed = exitPlan("x1", { plan: "# P" }, { content: "User has approved your plan.", isError: false });
    const denied = exitPlan("x2", { plan: "# P" }, { content: "Denied by the user.", isError: true });
    const states = planPresentations([user(), allowed, user(), denied], {}).map((p) => [p.toolUseId, p.state, p.requestId]);
    expect(states).toEqual([
      ["x1", "approved", null],
      ["x2", "rejected", null],
    ]);
  });

  it("reads a codex proposed plan inline, approved only by a later approval turn", () => {
    const proposed = tool("c1", "ProposedPlan", { plan: "# Codex plan" }, { content: "Plan proposed", isError: false });
    expect(planPresentations([user(), proposed], {})[0]).toMatchObject({ key: INLINE_PLAN_KEY, state: "proposed" });
    expect(planPresentations([user(), proposed, user("rename phase 2")], {})[0].state).toBe("proposed");
    expect(planPresentations([user(), proposed, user("Approved. Build it.")], {})[0].state).toBe("approved");
  });

  it("points OpenCode's plan_exit at the plan it wrote", () => {
    const items = [user(), write("w1", "/repo/.opencode/plans/x.md"), tool("e1", "plan_exit", {}, null)];
    expect(planPresentations(items, {})[0]).toMatchObject({ tool: "plan_exit", key: "/repo/.opencode/plans/x.md", state: "pending" });
  });

  it("ignores a plan_exit with no plan written", () => {
    expect(planPresentations([user(), tool("e1", "plan_exit", {}, null)], {})).toEqual([]);
  });
});

describe("presentationFor", () => {
  it("returns the newest presentation of a plan", () => {
    const items = [user(), exitPlan("x1", { plan: "a", planFilePath: CLAUDE_PLAN }, { content: "no", isError: true }), user(), exitPlan("x2", { plan: "b", planFilePath: CLAUDE_PLAN })];
    expect(presentationFor(items, {}, CLAUDE_PLAN)?.toolUseId).toBe("x2");
    expect(presentationFor(items, {}, THOUGHTS_PLAN)).toBeNull();
  });
});

describe("planKeyForSession", () => {
  it("prefers the newest presentation or finished write", () => {
    expect(planKeyForSession([user(), write("w1", THOUGHTS_PLAN), exitPlan("x1", { plan: "# P", planFilePath: CLAUDE_PLAN })], "plan", {})).toBe(CLAUDE_PLAN);
    expect(planKeyForSession([user(), exitPlan("x1", { plan: "# P" }), write("w1", THOUGHTS_PLAN)], "plan", {})).toBe(THOUGHTS_PLAN);
  });

  it("returns the inline key for a codex plan", () => {
    const proposed = tool("c1", "ProposedPlan", { plan: "# Codex" });
    expect(planKeyForSession([user(), proposed], "plan", {})).toBe(INLINE_PLAN_KEY);
  });

  it("counts a pending ExitPlanMode, since that is when the user approves it", () => {
    expect(planKeyForSession([user(), exitPlan("x1", { plan: "# P", planFilePath: CLAUDE_PLAN })], "plan", {})).toBe(CLAUDE_PLAN);
  });

  it("keeps an implement session on the plan it was given", () => {
    const first = user(`/implement_plan ${THOUGHTS_PLAN}`);
    expect(planKeyForSession([first, exitPlan("x1", { plan: "# P", planFilePath: CLAUDE_PLAN })], "implement", {})).toBe(THOUGHTS_PLAN);
  });
});
