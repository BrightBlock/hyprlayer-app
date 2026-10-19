// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DeriveGoalRequest, Goal, GoalCheck, GoalSpec } from "@/lib/bindings";
import { closeGoalSheet, useGoalSheet } from "@/lib/goalSheet";
import type { ShellTab } from "@/features/session/workstreamTabs";

import { BuildUntilMet } from "./BuildUntilMet";
import { parsePlanDoc } from "./planDoc";
import { trackPlan } from "./planTracker";

const b = vi.hoisted(() => ({
  getBootstrap: vi.fn(),
  claudeProbe: vi.fn(),
  deriveGoal: vi.fn(),
  startGoal: vi.fn(),
  getDoc: vi.fn(),
  patchDocFrontmatter: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({ commands: b }));

const s = vi.hoisted(() => ({ openGoalSession: vi.fn(), approvePlanAndBuild: vi.fn() }));
vi.mock("@/features/session/sessionStore", () => s);

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const PLAN = "/t/plans/widget.md";
const CONTENT = `---
status: draft
---
# Widget

## Phase 1: Model
- [ ] Add the model

### Success Criteria:
#### Automated Verification:
- [ ] Tests pass: \`cargo test -p widget\`

## Phase 2: View
- [ ] Add the view

### Success Criteria:
#### Automated Verification:
- [ ] Counter: \`test "$(cat counter.txt)" = 3\`
#### Manual Verification:
- [ ] The widget renders at narrow widths

## Phase 3: Polish
- [ ] Polish it
`;

const cmd = (command: string): GoalCheck => ({ kind: "command", command, timeoutSecs: 600, label: null });

const SPEC: GoalSpec = {
  condition: "Every phase of Widget is implemented and its automated criteria pass",
  workPrompt: `/implement_plan ${PLAN} all remaining phases consecutively`,
  source: "plan",
  sourceRef: PLAN,
  sourcePhase: null,
  checks: [
    cmd("cargo test -p widget"),
    cmd('test "$(cat counter.txt)" = 3'),
    cmd("pnpm test"),
    { kind: "plan_boxes", planPath: PLAN, phase: null },
  ],
  limits: { maxIterations: 10, maxCostUsd: 10, maxTokens: null, maxActiveSecs: 3600 },
};

const SHELL = { id: 1, title: "Plan", mode: "plan", draft: "", docs: [], attachments: [], paneId: "p1" } as ShellTab;

function goal(): Goal {
  return {
    id: "g1",
    sessionId: "s2",
    workstreamId: "w1",
    ...SPEC,
    status: "running",
    step: "working",
    pauseReason: null,
    endReason: null,
    iteration: 0,
    spentCostUsd: 0,
    spentTokens: 0,
    activeMs: 0,
    lastOutcome: null,
    lastReason: null,
    author: null,
    startedAt: 1,
    updatedAt: 1,
    endedAt: null,
  };
}

const lastDerive = (): DeriveGoalRequest | undefined => b.deriveGoal.mock.lastCall?.[0];

function renderFooter() {
  const tracker = trackPlan(parsePlanDoc(CONTENT, PLAN), [], null);
  const onError = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <BuildUntilMet workstreamId="w1" planPath={PLAN} tracker={tracker} shell={SHELL} onError={onError} />
    </QueryClientProvider>,
  );
  return { onError };
}

async function pick(label: string, option: string) {
  fireEvent.click(screen.getByRole("button", { name: label }));
  fireEvent.click(await screen.findByRole("option", { name: option }));
}

beforeEach(() => {
  b.getBootstrap.mockReturnValue(ok({ settings: {}, binaries: [] }));
  b.claudeProbe.mockReturnValue(ok({ models: null }));
  b.deriveGoal.mockImplementation(() => ok(SPEC));
  b.startGoal.mockReturnValue(ok(goal()));
  b.getDoc.mockReturnValue(ok({ content: CONTENT, mtimeMs: 5 }));
  b.patchDocFrontmatter.mockReturnValue(ok(null));
});
afterEach(() => {
  cleanup();
  closeGoalSheet();
  vi.clearAllMocks();
});

describe("BuildUntilMet", () => {
  it("scope and target", async () => {
    renderFooter();
    await waitFor(() => expect(lastDerive()?.phases).toEqual([]));
    expect(lastDerive()).toMatchObject({ workstreamId: "w1", planPath: PLAN, source: "plan" });

    fireEvent.click(screen.getByRole("radio", { name: "Phases" }));
    await pick("To phase", "Phase 2");
    await waitFor(() => expect(lastDerive()?.phases).toEqual(["1", "2"]));

    fireEvent.click(screen.getByRole("radio", { name: "Phase" }));
    await pick("Phase in scope", "Phase 2");
    await waitFor(() => expect(lastDerive()?.phases).toEqual(["2"]));

    fireEvent.click(screen.getByRole("radio", { name: "All phases" }));
    await waitFor(() => expect(lastDerive()?.phases).toEqual([]));

    fireEvent.click(await screen.findByRole("button", { name: "Approve & build until met" }));
    await waitFor(() => expect(s.openGoalSession).toHaveBeenCalled());
    expect(b.patchDocFrontmatter).toHaveBeenCalledWith(
      PLAN,
      { set: [{ key: "status", value: { kind: "str", value: "active" } }] },
      5,
    );
    expect(b.startGoal).toHaveBeenCalledWith({
      target: expect.objectContaining({
        kind: "new_session",
        workstreamId: "w1",
        stage: "implement",
        attachedDocs: [PLAN],
      }),
      spec: SPEC,
    });
    const [started, request] = s.openGoalSession.mock.calls[0];
    expect(started.id).toBe("g1");
    expect(request.target).toMatchObject({ newSession: { stage: "implement", attachedDocs: [PLAN] } });
  });

  it("every derived command is shown", async () => {
    renderFooter();
    for (const check of SPEC.checks) {
      if (check.kind === "command") expect(await screen.findByText(check.command)).toBeTruthy();
    }
    expect(screen.getByText(/The widget renders at narrow widths/)).toBeTruthy();
  });

  it("Review N commands hands the sheet that spec", async () => {
    renderFooter();
    fireEvent.click(await screen.findByRole("button", { name: "Review 3 commands" }));
    const request = useGoalSheet.getState().request;
    expect(request?.spec).toEqual(SPEC);
    expect(request?.target).toEqual({ newSession: { stage: "implement", attachedDocs: [PLAN] } });
    expect(request?.manual).toEqual(["The widget renders at narrow widths"]);
    expect(b.deriveGoal).toHaveBeenCalledTimes(1);
  });
});
