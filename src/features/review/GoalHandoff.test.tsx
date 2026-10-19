// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Goal, GoalSource } from "@/lib/bindings";

import { GoalHandoff } from "./GoalHandoff";
import { useValidateStepStore } from "./validateStep";

const m = vi.hoisted(() => ({
  findWorkstreamForNotify: vi.fn(),
  useWorkstreams: vi.fn(),
  git: { canShip: true, shipping: false, doShip: vi.fn() },
  validatePlanInNewSession: vi.fn(),
  openReview: vi.fn(),
}));
vi.mock("@/lib/notifications", () => ({ findWorkstreamForNotify: m.findWorkstreamForNotify }));
vi.mock("@/lib/queries", () => ({ useWorkstreams: m.useWorkstreams }));
vi.mock("@/features/workstreams/useGitActions", () => ({ useWorkstreamGitActions: () => m.git }));
vi.mock("@/features/session/sessionStore", () => ({ validatePlanInNewSession: m.validatePlanInNewSession }));
vi.mock("@/lib/appNav", () => ({ openReview: m.openReview }));

const PLAN = "/t/plans/widget.md";
const VIEW = { item: { id: "w1", projectPath: "/p" } };

function goal(source: GoalSource, sourceRef: string | null): Goal {
  return {
    id: "g1",
    sessionId: "impl1",
    workstreamId: "w1",
    condition: "done",
    workPrompt: "work",
    source,
    sourceRef,
    sourcePhase: null,
    checks: [],
    limits: { maxIterations: 10, maxCostUsd: 10, maxTokens: null, maxActiveSecs: 3600 },
    status: "met",
    step: null,
    pauseReason: null,
    endReason: "checks_passed",
    iteration: 2,
    spentCostUsd: 0.5,
    spentTokens: 0,
    activeMs: 1000,
    lastOutcome: "pass",
    lastReason: null,
    author: null,
    startedAt: 1,
    updatedAt: 2,
    endedAt: 2,
  };
}

function renderHandoff(g: Goal) {
  render(<GoalHandoff sessionId="impl1" workstreamId="w1" goal={g} />);
}

beforeEach(() => {
  m.git.canShip = true;
  m.findWorkstreamForNotify.mockReturnValue({ projectPath: "/p" });
  m.useWorkstreams.mockReturnValue({ data: [VIEW] });
  m.validatePlanInNewSession.mockResolvedValue({ id: "val1" });
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useValidateStepStore.setState({ byImplement: {} });
});

describe("GoalHandoff", () => {
  it("offers Validate plan only for plan sources", async () => {
    renderHandoff(goal("composer", null));
    expect(screen.queryByRole("button", { name: "Validate plan" })).toBeNull();
    expect(screen.getByRole("button", { name: "Review changes" })).toBeTruthy();
    cleanup();

    renderHandoff(goal("validation", PLAN));
    expect(screen.queryByRole("button", { name: "Validate plan" })).toBeNull();
    cleanup();

    renderHandoff(goal("plan_phase", PLAN));
    fireEvent.click(screen.getByRole("button", { name: "Validate plan" }));
    await waitFor(() => expect(m.validatePlanInNewSession).toHaveBeenCalledWith("w1", PLAN, "impl1"));
    await waitFor(() =>
      expect(useValidateStepStore.getState().byImplement.impl1).toEqual({ kind: "agent", sessionId: "val1" }),
    );
  });

  it("disables Ship when the workstream cannot ship", () => {
    m.git.canShip = false;
    renderHandoff(goal("plan", PLAN));
    expect((screen.getByRole("button", { name: /Ship/ }) as HTMLButtonElement).disabled).toBe(true);
    cleanup();

    m.git.canShip = true;
    renderHandoff(goal("plan", PLAN));
    const ship = screen.getByRole("button", { name: /Ship/ }) as HTMLButtonElement;
    expect(ship.disabled).toBe(false);
    fireEvent.click(ship);
    expect(m.git.doShip).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Review changes" }));
    expect(m.openReview).toHaveBeenCalledWith({ projectPath: "/p", workstreamId: "w1" });
  });
});
