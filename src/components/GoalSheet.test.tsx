// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Bootstrap, Goal, GoalCheck, GoalSpec, Session } from "@/lib/bindings";
import { closeGoalSheet, openGoalSheet, type GoalSheetRequest } from "@/lib/goalSheet";

import { GoalSheetHost } from "./GoalSheet";

const b = vi.hoisted(() => ({
  getBootstrap: vi.fn(),
  listSessions: vi.fn(),
  deriveGoal: vi.fn(),
  startGoal: vi.fn(),
  updateGoal: vi.fn(),
  resumeGoal: vi.fn(),
  getGoal: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({ commands: b }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const cmd = (command: string): GoalCheck => ({ kind: "command", command, timeoutSecs: 600 });
const EVALUATOR: GoalCheck = { kind: "evaluator", model: null };

const SPEC: GoalSpec = {
  condition: "counter.txt holds 3",
  workPrompt: "counter.txt holds 3",
  source: "composer",
  sourceRef: null,
  sourcePhase: null,
  checks: [cmd("cargo test"), cmd("pnpm test"), EVALUATOR],
  limits: { maxIterations: 10, maxCostUsd: 10, maxTokens: null, maxActiveSecs: 3600 },
};

const SESSION = {
  id: "s1",
  workstreamId: "w1",
  stage: "general",
  harness: "claude",
  model: "haiku",
  state: "running",
} as Session;

function bootstrap(claude: "ok" | "missing" = "ok"): Bootstrap {
  return {
    settings: {},
    binaries: [
      {
        kind: "claude",
        name: "claude",
        path: claude === "ok" ? "/usr/local/bin/claude" : null,
        version: null,
        state: { state: claude },
        optional: false,
        overridden: false,
      },
    ],
  } as unknown as Bootstrap;
}

function goal(over: Partial<Goal> = {}): Goal {
  return {
    id: "g1",
    sessionId: "s1",
    workstreamId: "w1",
    condition: SPEC.condition,
    workPrompt: SPEC.workPrompt,
    source: "composer",
    sourceRef: null,
    sourcePhase: null,
    checks: SPEC.checks,
    limits: SPEC.limits,
    status: "running",
    step: "working",
    pauseReason: null,
    endReason: null,
    iteration: 2,
    spentCostUsd: 1.2,
    spentTokens: 0,
    activeMs: 60_000,
    lastOutcome: null,
    lastReason: null,
    author: null,
    startedAt: 1,
    updatedAt: 2,
    endedAt: null,
    ...over,
  };
}

function startRequest(over: Partial<GoalSheetRequest> = {}): GoalSheetRequest {
  return {
    mode: "start",
    workstreamId: "w1",
    source: "composer",
    text: SPEC.condition,
    target: { sessionId: "s1" },
    ...over,
  };
}

function renderSheet(request: GoalSheetRequest, claude: "ok" | "missing" = "ok") {
  b.getBootstrap.mockReturnValue(ok(bootstrap(claude)));
  b.listSessions.mockReturnValue(ok([SESSION]));
  b.deriveGoal.mockReturnValue(ok(SPEC));
  b.startGoal.mockReturnValue(ok(goal()));
  b.updateGoal.mockReturnValue(ok(goal()));
  b.resumeGoal.mockReturnValue(ok(goal()));
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <GoalSheetHost />
    </QueryClientProvider>,
  );
  act(() => openGoalSheet(request));
}

const submit = () => screen.findByTestId("goal-sheet-submit") as Promise<HTMLButtonElement>;
const toggle = (name: string | RegExp) =>
  screen.findByRole("switch", { name }) as Promise<HTMLInputElement>;

beforeEach(() => closeGoalSheet());

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("GoalSheet", () => {
  it("the gate rule", async () => {
    renderSheet(startRequest({ spec: { ...SPEC, checks: [cmd("cargo test"), EVALUATOR] } }));
    const start = await submit();
    await waitFor(() => expect(start.disabled).toBe(false));

    fireEvent.click(await toggle("Run cargo test"));
    fireEvent.click(await toggle("Evaluator"));
    await waitFor(() => expect(start.disabled).toBe(true));
    expect(start.getAttribute("title")).toBe(
      "A goal needs at least one command, evaluator or validation check.",
    );

    fireEvent.click(await toggle("Run cargo test"));
    await waitFor(() => expect(start.disabled).toBe(false));
  });

  it("unticked commands are dropped", async () => {
    renderSheet(startRequest({ spec: SPEC }));
    fireEvent.click(await toggle("Run pnpm test"));
    const start = await submit();
    await waitFor(() => expect(start.disabled).toBe(false));
    fireEvent.click(start);

    await waitFor(() => expect(b.startGoal).toHaveBeenCalledTimes(1));
    const request = b.startGoal.mock.calls[0][0];
    const commands = request.spec.checks
      .filter((c: GoalCheck) => c.kind === "command")
      .map((c: GoalCheck & { kind: "command" }) => c.command);
    expect(commands).toEqual(["cargo test"]);
    expect(request.target).toEqual({ kind: "session", sessionId: "s1" });
  });

  it("a missing evaluator binary", async () => {
    renderSheet(startRequest({ spec: SPEC }), "missing");
    const evaluator = await toggle("Evaluator");
    await waitFor(() => expect(evaluator.disabled).toBe(true));
    expect(evaluator.checked).toBe(false);
    expect(screen.getByText("claude is not installed")).toBeTruthy();

    const start = await submit();
    await waitFor(() => expect(start.disabled).toBe(false));
    fireEvent.click(start);
    await waitFor(() => expect(b.startGoal).toHaveBeenCalledTimes(1));
    const kinds = b.startGoal.mock.calls[0][0].spec.checks.map((c: GoalCheck) => c.kind);
    expect(kinds).not.toContain("evaluator");
  });

  describe("mode picks the mutation", () => {
    it("edit updates the goal", async () => {
      b.getGoal.mockReturnValue(ok({ goal: goal(), iterations: [] }));
      renderSheet({ ...startRequest(), mode: "edit", goalId: "g1" });
      const save = await submit();
      await waitFor(() => expect(save.disabled).toBe(false));
      fireEvent.click(save);

      await waitFor(() => expect(b.updateGoal).toHaveBeenCalledTimes(1));
      expect(b.updateGoal.mock.calls[0][0]).toBe("g1");
      expect(b.resumeGoal).not.toHaveBeenCalled();
      expect(b.startGoal).not.toHaveBeenCalled();
    });

    it("extend updates, then resumes", async () => {
      b.getGoal.mockReturnValue(
        ok({ goal: goal({ status: "exhausted", step: null, endReason: "iteration_limit" }), iterations: [] }),
      );
      renderSheet({ ...startRequest(), mode: "extend", goalId: "g1" });
      const extend = await submit();
      await waitFor(() => expect(extend.disabled).toBe(false));
      fireEvent.click(extend);

      await waitFor(() => expect(b.resumeGoal).toHaveBeenCalledTimes(1));
      expect(b.updateGoal).toHaveBeenCalledTimes(1);
      expect(b.updateGoal.mock.invocationCallOrder[0]).toBeLessThan(
        b.resumeGoal.mock.invocationCallOrder[0],
      );
      expect(b.resumeGoal.mock.calls[0][0]).toBe("g1");
      expect(b.startGoal).not.toHaveBeenCalled();
    });

    it("start again with a prefilled spec starts a new goal", async () => {
      renderSheet(startRequest({ spec: SPEC, goalId: "g1" }));
      const start = await submit();
      await waitFor(() => expect(start.disabled).toBe(false));
      fireEvent.click(start);

      await waitFor(() => expect(b.startGoal).toHaveBeenCalledTimes(1));
      expect(b.updateGoal).not.toHaveBeenCalled();
      expect(b.resumeGoal).not.toHaveBeenCalled();
    });
  });

  it("a handed spec is kept", async () => {
    const handed: GoalSpec = {
      ...SPEC,
      source: "plan_phase",
      sourceRef: "thoughts/shared/plans/widget.md",
      sourcePhase: "2",
      checks: [cmd("cargo test -p widget")],
    };
    renderSheet(startRequest({ source: "plan_phase", spec: handed }));
    const start = await submit();
    await waitFor(() => expect(start.disabled).toBe(false));
    fireEvent.click(start);

    await waitFor(() => expect(b.startGoal).toHaveBeenCalledTimes(1));
    expect(b.deriveGoal).not.toHaveBeenCalled();
    const spec = b.startGoal.mock.calls[0][0].spec;
    expect(spec.checks).toEqual(handed.checks);
    expect(spec.sourceRef).toBe(handed.sourceRef);
    expect(spec.sourcePhase).toBe("2");
  });

  it("withValidation adds the check", async () => {
    const plan = "thoughts/shared/plans/widget.md";
    renderSheet(startRequest({ source: "plan", text: undefined, planPath: plan, withValidation: true }));
    expect((await toggle("Validation")).checked).toBe(true);
    const start = await submit();
    await waitFor(() => expect(start.disabled).toBe(false));
    fireEvent.click(start);

    await waitFor(() => expect(b.startGoal).toHaveBeenCalledTimes(1));
    expect(b.deriveGoal).toHaveBeenCalledTimes(1);
    const checks = b.startGoal.mock.calls[0][0].spec.checks;
    expect(checks.at(-1)).toEqual({ kind: "validation", planPath: plan });
  });
});
