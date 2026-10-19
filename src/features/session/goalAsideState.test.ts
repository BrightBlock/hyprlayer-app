import { describe, expect, it } from "vitest";

import { claimGoalAutoOpen, resolveAsideView } from "./goalAsideState";

describe("resolveAsideView", () => {
  it("a persisted goal view with no goal falls back to Plan, else Changes", () => {
    expect(resolveAsideView("goal", { plan: true, goal: false })).toBe("plan");
    expect(resolveAsideView("goal", { plan: false, goal: false })).toBe("changes");
  });

  it("a stale plan view with a goal and no plan resolves to Goal", () => {
    expect(resolveAsideView("plan", { plan: false, goal: true })).toBe("goal");
  });

  it("keeps an available pick", () => {
    expect(resolveAsideView("goal", { plan: true, goal: true })).toBe("goal");
    expect(resolveAsideView("changes", { plan: true, goal: true })).toBe("changes");
  });
});

describe("goal view auto-open", () => {
  const running = { id: "g1", status: "running" as const };

  it("opens once per goal id", () => {
    const opened = new Set<string>();
    expect(claimGoalAutoOpen(opened, running, { plan: false, chosen: false })).toBe(true);
    expect(claimGoalAutoOpen(opened, running, { plan: false, chosen: false })).toBe(false);
    expect(claimGoalAutoOpen(opened, { id: "g2", status: "running" }, { plan: false, chosen: false })).toBe(true);
  });

  it("never opens beside a plan or over a pick made this run", () => {
    const opened = new Set<string>();
    expect(claimGoalAutoOpen(opened, running, { plan: true, chosen: false })).toBe(false);
    expect(claimGoalAutoOpen(opened, running, { plan: false, chosen: true })).toBe(false);
  });

  it("opens only for a running goal", () => {
    const opened = new Set<string>();
    expect(claimGoalAutoOpen(opened, null, { plan: false, chosen: false })).toBe(false);
    expect(claimGoalAutoOpen(opened, { id: "g1", status: "paused" }, { plan: false, chosen: false })).toBe(false);
  });
});
