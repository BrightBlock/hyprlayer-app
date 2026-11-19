import { describe, expect, it } from "vitest";

import type { Goal, GoalChanged, GoalCheckResult, GoalIteration, GoalStatus, GoalStep } from "@/lib/bindings";
import {
  commandOutcomes,
  criterionOutcome,
  goalActivity,
  goalChip,
  goalControls,
  goalDraft,
  goalNotice,
  liveGoalForPlan,
  type GoalControl,
} from "./goals";

const NOW = 2_000;

const goal = (over: Partial<Goal> = {}): Goal => ({
  id: "g1",
  sessionId: "s1",
  workstreamId: "w1",
  condition: "all tests pass",
  workPrompt: "make the tests pass",
  source: "composer",
  sourceRef: null,
  sourcePhase: null,
  checks: [{ kind: "command", command: "pnpm test", timeoutSecs: 600, label: null }],
  limits: { maxIterations: 10, maxCostUsd: 10, maxTokens: null, maxActiveSecs: 3600 },
  status: "running",
  step: "checking",
  pauseReason: null,
  endReason: null,
  iteration: 3,
  spentCostUsd: 1.24,
  spentTokens: 0,
  activeMs: 724_000,
  lastOutcome: null,
  lastReason: null,
  author: null,
  startedAt: 1_000,
  updatedAt: NOW,
  endedAt: null,
  ...over,
});

const changed = (over: Partial<GoalChanged> = {}): GoalChanged => ({
  goalId: "g1",
  sessionId: "s1",
  workstreamId: "w1",
  status: "running",
  step: "working",
  pauseReason: null,
  endReason: null,
  reason: null,
  iteration: 1,
  maxIterations: 10,
  ...over,
});

describe("controls per status", () => {
  const cases: Array<[GoalStatus, GoalControl[]]> = [
    ["running", ["pause", "stop", "edit"]],
    ["paused", ["resume", "stop", "edit"]],
    ["exhausted", ["extend", "startAgain"]],
    ["failed", ["startAgain"]],
    ["stopped", ["startAgain"]],
    ["met", []],
  ];

  it.each(cases)("%s offers %j", (status, controls) => {
    expect(goalControls(goal({ status }))).toEqual(controls);
  });

});

describe("notices per transition", () => {
  it("met", () => {
    const notice = goalNotice(changed(), changed({ status: "met", step: null, endReason: "checks_passed" }));
    expect(notice?.title).toBe("Goal met");
  });

  it("exhausted", () => {
    const notice = goalNotice(
      changed(),
      changed({ status: "exhausted", step: null, endReason: "iteration_limit" }),
    );
    expect(notice?.title).toBe("Goal stopped at its limit");
  });

  it("failed carries the end reason's label and the payload's reason", () => {
    const notice = goalNotice(
      changed(),
      changed({ status: "failed", step: null, endReason: "stalled", reason: "no diff in 3 iterations" }),
    );
    expect(notice?.title).toBe("Goal failed: no progress");
    expect(notice?.body).toContain("no diff in 3 iterations");
  });

  it("a pause for a non-user reason carries its label and the payload's reason", () => {
    const notice = goalNotice(
      changed(),
      changed({
        status: "paused",
        step: null,
        pauseReason: "needs_human",
        reason: "NEEDS-HUMAN: which database?",
      }),
    );
    expect(notice?.title).toBe("Goal needs you: a decision is needed");
    expect(notice?.body).toContain("NEEDS-HUMAN: which database?");
  });

  it("entering the waiting step names the tool or question", () => {
    const notice = goalNotice(changed(), changed({ step: "waiting", reason: "Bash: rm -rf build" }));
    expect(notice?.title).toBe("Goal needs you");
    expect(notice?.body).toBe("Bash: rm -rf build");
  });

  it("a first event with no previous payload still notices", () => {
    expect(goalNotice(null, changed({ status: "met", step: null }))?.title).toBe("Goal met");
  });

  it("is null for a pause by the user", () => {
    expect(goalNotice(changed(), changed({ status: "paused", step: null, pauseReason: "user" }))).toBeNull();
  });

  it("is null for a stop", () => {
    expect(
      goalNotice(changed(), changed({ status: "stopped", step: null, endReason: "stopped_by_user" })),
    ).toBeNull();
  });

  it("is null for a step change", () => {
    expect(goalNotice(changed(), changed({ step: "checking" }))).toBeNull();
  });

  it("is null for no change", () => {
    expect(goalNotice(changed(), changed())).toBeNull();
    const met = changed({ status: "met", step: null });
    expect(goalNotice(met, met)).toBeNull();
    const waiting = changed({ step: "waiting", reason: "Bash" });
    expect(goalNotice(waiting, waiting)).toBeNull();
  });
});

describe("goal drafts", () => {
  it.each([
    ["/goal x", "x"],
    ["/goal", ""],
    ["  /goal x", "x"],
    // What accepting the composer's `/goal` suggestion leaves in the draft.
    ["/goal ", ""],
  ])("%j is a goal draft", (text, condition) => {
    expect(goalDraft(text)).toBe(condition);
  });

  it.each(["/goals", "x /goal", "/goalx"])("%j is not a goal draft", (text) => {
    expect(goalDraft(text)).toBeNull();
  });
});

describe("chip per harness", () => {
  it("a Claude goal's chip reads dollars", () => {
    const g = goal();
    expect(goalChip(g, NOW)).toBe("3/10 · checking · $1.24 · 12:04");
  });

  it("a Codex goal's chip reads tokens", () => {
    const g = goal({
      limits: { maxIterations: 10, maxCostUsd: null, maxTokens: 2_000_000, maxActiveSecs: 3600 },
      spentCostUsd: 0,
      spentTokens: 412_000,
    });
    const chip = goalChip(g, NOW);
    expect(chip).toContain("412.0k tokens");
    expect(chip).not.toContain("$");
  });
});

describe("goalActivity", () => {
  const brief = (status: GoalStatus, step: GoalStep | null, pauseReason: Goal["pauseReason"] = null) => ({
    status,
    step,
    pauseReason,
  });

  it.each(["working", "checking", "resuming", "pausing", "stopping"] as const)(
    "a running goal at %s is working",
    (step) => {
      expect(goalActivity(brief("running", step))).toBe("working");
    },
  );

  it("a running goal between iterations is working", () => {
    expect(goalActivity(brief("running", null))).toBe("working");
  });

  it("a running goal waiting on an ask needs you", () => {
    expect(goalActivity(brief("running", "waiting"))).toBe("needs-you");
  });

  it("a pause for a non-user reason needs you", () => {
    expect(goalActivity(brief("paused", null, "needs_human"))).toBe("needs-you");
    expect(goalActivity(brief("paused", null, "turn_failures"))).toBe("needs-you");
  });

  it("a pause by the user falls back to the session's own rules", () => {
    expect(goalActivity(brief("paused", null, "user"))).toBeNull();
  });

  it("a met goal is ready for review", () => {
    expect(goalActivity(brief("met", null))).toBe("review");
  });

  it.each(["exhausted", "failed", "stopped"] as const)("a %s goal falls back", (status) => {
    expect(goalActivity(brief(status, null))).toBeNull();
  });

  it("no goal falls back", () => {
    expect(goalActivity(null)).toBeNull();
    expect(goalActivity(undefined)).toBeNull();
  });
});

describe("criterion outcomes", () => {
  const PLAN = "/t/plans/widget.md";
  const planGoal = goal({
    source: "plan",
    sourceRef: PLAN,
    checks: [
      { kind: "command", command: "pnpm test", timeoutSecs: 600, label: "Unit tests" },
      { kind: "command", command: "pnpm test:e2e", timeoutSecs: 600, label: null },
      { kind: "plan_boxes", planPath: PLAN, phase: null },
    ],
  });
  const result = (label: string, outcome: GoalCheckResult["outcome"], exitCode: number | null): GoalCheckResult => ({
    kind: "command",
    label,
    outcome,
    exitCode,
    durationMs: 10,
    outputTail: "",
    reason: null,
  });
  const iteration = (n: number, results: GoalCheckResult[]): GoalIteration => ({
    goalId: "g1",
    n,
    clientMessageId: `m${n}`,
    startedAt: n,
    endedAt: n + 1,
    turnStatus: null,
    costUsd: null,
    tokens: null,
    outcome: null,
    reason: null,
    results,
    progressHash: null,
    note: null,
  });
  const detail = {
    goal: planGoal,
    iterations: [
      iteration(1, [result("Unit tests", "fail", 1), result("pnpm test:e2e", "fail", 101)]),
      iteration(2, [result("Unit tests", "pass", 0), result("pnpm test:e2e", "fail", 2)]),
      iteration(3, []),
    ],
  };
  const outcomes = commandOutcomes(detail);

  it("takes the newest result of the criterion's own command", () => {
    expect(criterionOutcome({ title: "Unit tests: pnpm test", group: "automated" }, outcomes)).toEqual({
      command: "pnpm test",
      outcome: "pass",
      exitCode: 0,
    });
    expect(criterionOutcome({ title: "E2E: pnpm test:e2e", group: "automated" }, outcomes)).toEqual({
      command: "pnpm test:e2e",
      outcome: "fail",
      exitCode: 2,
    });
    expect(criterionOutcome({ title: "Types: pnpm typecheck", group: "automated" }, outcomes)).toBeNull();
  });

  it("ignores manual rows", () => {
    expect(criterionOutcome({ title: "Run pnpm test by hand", group: "manual" }, outcomes)).toBeNull();
  });

  it("drops a result whose label no longer matches its check", () => {
    const relabelled = commandOutcomes({ ...detail, iterations: [iteration(1, [result("Old label", "fail", 1)])] });
    expect(criterionOutcome({ title: "Unit tests: pnpm test", group: "automated" }, relabelled)).toBeNull();
  });

  it("liveGoalForPlan ignores terminal goals and other plans' goals", () => {
    const other = goal({ id: "other", sourceRef: "/t/plans/other.md" });
    const ended = goal({ id: "ended", sourceRef: PLAN, status: "met" });
    const live = goal({ id: "live", sourceRef: PLAN, status: "paused" });
    expect(liveGoalForPlan([other, ended, live], PLAN)?.id).toBe("live");
    expect(liveGoalForPlan([other, ended], PLAN)).toBeNull();
    expect(liveGoalForPlan(undefined, PLAN)).toBeNull();
  });
});
