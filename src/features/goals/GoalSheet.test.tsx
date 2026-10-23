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
  getDoc: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({ commands: b }));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const cmd = (command: string): GoalCheck => ({ kind: "command", command, timeoutSecs: 600 });
const EVALUATOR: GoalCheck = { kind: "evaluator", model: null };
const PLAN_PATH = "/t/plans/api.md";

const phased = (label: string, command: string, phases: string[], cwd: string): GoalCheck => ({
  kind: "command",
  command,
  timeoutSecs: 600,
  label,
  cwd,
  phases,
});

const SPEC: GoalSpec = {
  condition: "counter.txt holds 3",
  workPrompt: "counter.txt holds 3",
  source: "composer",
  sourceRef: null,
  sourcePhase: null,
  checks: [cmd("cargo test"), cmd("pnpm test"), EVALUATOR],
  limits: { maxIterations: 10, maxCostUsd: 10, maxTokens: null, maxActiveSecs: 3600 },
};

const PLAN_SPEC: GoalSpec = {
  ...SPEC,
  condition: "Every phase of api is implemented and its automated criteria pass",
  workPrompt: `/implement_plan ${PLAN_PATH} all remaining phases consecutively`,
  source: "plan",
  sourceRef: PLAN_PATH,
  checks: [
    phased("Existing suites pass", "uv run pytest", ["1", "5"], "tools/web"),
    phased("Lint passes", "npm run lint", ["2"], "tools/web/web"),
    phased("Script parses", "bash -n infra/scripts/entra.sh", ["5"], "tools/web"),
    { kind: "plan_boxes", planPath: PLAN_PATH, phase: null },
  ],
};

const PLAN_TEXT = [
  "---",
  "title: api",
  "type: plan",
  "status: active",
  "---",
  "",
  "# API",
  "",
  "## Phase 1: OpenAPI spec",
  "",
  "### Success Criteria:",
  "",
  "#### Automated Verification (from `tools/web/`):",
  "- [x] Existing suites pass: `uv run pytest`",
  "",
  "## Phase 2: Swagger UI",
  "",
  "### Success Criteria:",
  "",
  "#### Automated Verification (from `tools/web/web/`):",
  "- [x] Lint passes: `npm run lint`",
  "",
  "## Phase 5: Power BI-compatible authentication",
  "",
  "### Success Criteria:",
  "",
  "#### Automated Verification (from `tools/web/`):",
  "- [ ] Existing suites pass: `uv run pytest`",
  "- [ ] Script parses: `bash -n infra/scripts/entra.sh`",
  "",
  "#### Manual Verification:",
  "- [ ] Power BI Desktop loads rows",
  "",
].join("\n");

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
  b.getDoc.mockReturnValue(ok({ content: PLAN_TEXT, mtimeMs: 1, meta: { backend: "git" } }));
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
const openPage = async (name: RegExp) => fireEvent.click(await screen.findByRole("button", { name }));

async function startedChecks(): Promise<GoalCheck[]> {
  const start = await submit();
  await waitFor(() => expect(start.disabled).toBe(false));
  fireEvent.click(start);
  await waitFor(() => expect(b.startGoal).toHaveBeenCalledTimes(1));
  return b.startGoal.mock.calls[0][0].spec.checks;
}

const commandsOf = (checks: GoalCheck[]) =>
  checks.flatMap((c) => (c.kind === "command" ? [c.command] : []));

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

    await openPage(/^Checks/);
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
    await openPage(/^Checks/);
    fireEvent.click(await toggle("Run pnpm test"));

    expect(commandsOf(await startedChecks())).toEqual(["cargo test"]);
    expect(b.startGoal.mock.calls[0][0].target).toEqual({ kind: "session", sessionId: "s1" });
  });

  it("a missing evaluator binary", async () => {
    renderSheet(startRequest({ spec: SPEC }), "missing");
    await openPage(/^Checks/);
    const evaluator = await toggle("Evaluator");
    await waitFor(() => expect(evaluator.disabled).toBe(true));
    expect(evaluator.checked).toBe(false);
    expect(screen.getByText("claude is not installed")).toBeTruthy();

    const kinds = (await startedChecks()).map((c) => c.kind);
    expect(kinds).not.toContain("evaluator");
  });

  describe("a plan's phases", () => {
    it("open on the first unfinished phase, which lists what it shares with earlier ones", async () => {
      renderSheet(startRequest({ source: "plan", spec: PLAN_SPEC }));
      expect(await screen.findByText("Power BI-compatible authentication")).toBeTruthy();
      expect(screen.getByText("bash -n infra/scripts/entra.sh")).toBeTruthy();
      expect(screen.getByText("Shared with earlier phases")).toBeTruthy();
      expect(screen.getByRole("button", { name: /^Existing suites pass/ })).toBeTruthy();
    });

    it("turning a phase off drops its own checks and keeps one an earlier phase shares", async () => {
      renderSheet(startRequest({ source: "plan", spec: PLAN_SPEC }));
      await openPage(/^Phase 5/);
      fireEvent.click(await toggle("Run Phase 5 checks"));

      const checks = await startedChecks();
      expect(commandsOf(checks)).toEqual(["uv run pytest", "npm run lint"]);
      expect(checks[checks.length - 1]).toEqual({ kind: "plan_boxes", planPath: PLAN_PATH, phase: null });
    });

    it("a phase's directory applies to its own commands, normalized", async () => {
      renderSheet(startRequest({ source: "plan", spec: PLAN_SPEC }));
      await openPage(/^Phase 2/);
      fireEvent.change(await screen.findByRole("textbox", { name: "Phase 2 directory" }), {
        target: { value: "./tools/web/app/" },
      });

      const lint = (await startedChecks()).find((c) => c.kind === "command" && c.command === "npm run lint");
      expect(lint).toMatchObject({ cwd: "tools/web/app", phases: ["2"] });
    });

    it("a directory outside the worktree blocks the start", async () => {
      renderSheet(startRequest({ source: "plan", spec: PLAN_SPEC }));
      await openPage(/^Phase 5/);
      fireEvent.click(await screen.findByRole("button", { name: "Edit Script parses" }));
      fireEvent.change(await screen.findByRole("textbox", { name: "Script parses directory" }), {
        target: { value: "../elsewhere" },
      });

      const start = await submit();
      await waitFor(() => expect(start.disabled).toBe(true));
      expect(start.getAttribute("title")).toBe("A check's directory must be inside the worktree.");
    });
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

    const checks = await startedChecks();
    expect(b.deriveGoal).not.toHaveBeenCalled();
    expect(checks).toEqual([{ ...handed.checks[0], label: null, cwd: null, phases: [] }]);
    const spec = b.startGoal.mock.calls[0][0].spec;
    expect(spec.sourceRef).toBe(handed.sourceRef);
    expect(spec.sourcePhase).toBe("2");
  });

  it("withValidation adds the check", async () => {
    const plan = "thoughts/shared/plans/widget.md";
    renderSheet(startRequest({ source: "plan", text: undefined, planPath: plan, withValidation: true }));
    await openPage(/^Checks/);
    expect((await toggle("Validation")).checked).toBe(true);

    const checks = await startedChecks();
    expect(b.deriveGoal).toHaveBeenCalledTimes(1);
    expect(checks[checks.length - 1]).toEqual({ kind: "validation", planPath: plan });
  });
});
