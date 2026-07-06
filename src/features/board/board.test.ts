import { describe, expect, it } from "vitest";

import type { Stage, WorkstreamView } from "@/lib/bindings";
import { columnOf, groupByColumn } from "./board";

function makeView(
  over: Partial<{
    id: string;
    stage: Stage;
    prNumber: number | null;
    updatedAt: number | null;
  }> = {},
): WorkstreamView {
  return {
    item: {
      id: over.id ?? "ws-1",
      projectPath: "/project",
      title: "Test workstream",
      slug: "test",
      branch: "feature/test",
      worktreePath: "/worktrees/test",
      baseBranch: "main",
      worktreeMode: "isolated",
      stage: over.stage ?? "general",
      status: "active",
      prNumber: over.prNumber !== undefined ? over.prNumber : null,
      createdAt: null,
      updatedAt: over.updatedAt !== undefined ? over.updatedAt : null,
      archivedAt: null,
    },
    git: {
      exists: true,
      branch: "feature/test",
      dirty: 0,
      additions: 0,
      deletions: 0,
      untracked: 0,
      ahead: 0,
      behind: 0,
      aheadOfBase: 0,
      thoughts: "ok" as const,
    },
    seed: null,
  };
}

describe("columnOf", () => {
  it("maps 'general' stage to backlog", () => {
    expect(columnOf(makeView({ stage: "general" }))).toBe("backlog");
  });

  it("maps 'research' stage to research", () => {
    expect(columnOf(makeView({ stage: "research" }))).toBe("research");
  });

  it("maps 'plan' stage to plan", () => {
    expect(columnOf(makeView({ stage: "plan" }))).toBe("plan");
  });

  it("maps 'implement' stage to implement", () => {
    expect(columnOf(makeView({ stage: "implement" }))).toBe("implement");
  });

  it("maps 'validate' stage to validate", () => {
    expect(columnOf(makeView({ stage: "validate" }))).toBe("validate");
  });

  it("maps 'commit' stage to ship", () => {
    expect(columnOf(makeView({ stage: "commit" }))).toBe("ship");
  });

  it("maps 'ship' stage to ship", () => {
    expect(columnOf(makeView({ stage: "ship" }))).toBe("ship");
  });

  it("maps 'revise' stage to ship", () => {
    expect(columnOf(makeView({ stage: "revise" }))).toBe("ship");
  });

  it("maps an unknown stage to backlog", () => {
    // Simulates a NEWER backend: the wire union can't produce this today, but
    // columnOf must still fall back to backlog rather than crash.
    expect(columnOf(makeView({ stage: "unknown_future_stage" as Stage }))).toBe("backlog");
  });

  it("prNumber overrides stage — non-ship stage lands in ship", () => {
    expect(columnOf(makeView({ stage: "plan", prNumber: 42 }))).toBe("ship");
  });

  it("prNumber overrides stage — general/backlog stage lands in ship", () => {
    expect(columnOf(makeView({ stage: "general", prNumber: 1 }))).toBe("ship");
  });

  it("prNumber=null does not affect stage mapping", () => {
    expect(columnOf(makeView({ stage: "implement", prNumber: null }))).toBe("implement");
  });
});

describe("groupByColumn", () => {
  it("returns all six column keys even when the input is empty", () => {
    const grouped = groupByColumn([]);
    const keys = Object.keys(grouped).sort();
    expect(keys).toEqual(
      ["backlog", "implement", "plan", "research", "ship", "validate"].sort(),
    );
    for (const key of Object.keys(grouped)) {
      expect(grouped[key as keyof typeof grouped]).toEqual([]);
    }
  });

  it("puts workstreams into the correct column", () => {
    const views = [
      makeView({ id: "1", stage: "research" }),
      makeView({ id: "2", stage: "plan" }),
      makeView({ id: "3", stage: "general" }),
    ];
    const grouped = groupByColumn(views);
    expect(grouped.research.map((v) => v.item.id)).toEqual(["1"]);
    expect(grouped.plan.map((v) => v.item.id)).toEqual(["2"]);
    expect(grouped.backlog.map((v) => v.item.id)).toEqual(["3"]);
    expect(grouped.implement).toEqual([]);
    expect(grouped.validate).toEqual([]);
    expect(grouped.ship).toEqual([]);
  });

  it("sorts by newest updatedAt first within a column", () => {
    const views = [
      makeView({ id: "older", stage: "plan", updatedAt: 1000 }),
      makeView({ id: "newer", stage: "plan", updatedAt: 2000 }),
    ];
    const grouped = groupByColumn(views);
    expect(grouped.plan.map((v) => v.item.id)).toEqual(["newer", "older"]);
  });

  it("treats null updatedAt as 0 for ordering (named items sort before null)", () => {
    const views = [
      makeView({ id: "null-ts", stage: "plan", updatedAt: null }),
      makeView({ id: "has-ts", stage: "plan", updatedAt: 500 }),
    ];
    const grouped = groupByColumn(views);
    expect(grouped.plan.map((v) => v.item.id)).toEqual(["has-ts", "null-ts"]);
  });

  it("routes a workstream with prNumber to ship regardless of stage", () => {
    const views = [makeView({ id: "shipping", stage: "implement", prNumber: 99 })];
    const grouped = groupByColumn(views);
    expect(grouped.ship.map((v) => v.item.id)).toEqual(["shipping"]);
    expect(grouped.implement).toEqual([]);
  });
});
