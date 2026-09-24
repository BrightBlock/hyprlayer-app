// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PlanCardsContext, registerPlanCard, type PlanCardProps } from "./planCardSlot";
import { planRevisions } from "./planRevisions";
import type { TranscriptItem } from "./transcript";
import { TranscriptView } from "./TranscriptView";

vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn() }));

registerPlanCard(({ toolUseId, superseded }: PlanCardProps) => (
  <div data-testid="plan-card">
    {toolUseId}:{superseded ? "superseded" : "latest"}
  </div>
));

afterEach(cleanup);

const PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";
const user: TranscriptItem = { kind: "user", id: "u1", text: "/create_plan", stage: null, attachments: [], createdAt: null, promptName: null };
const tool = (toolUseId: string, name: string, input: object, extra: Partial<TranscriptItem> = {}): TranscriptItem =>
  ({
    kind: "tool",
    id: `i-${toolUseId}`,
    toolUseId,
    name,
    input: JSON.stringify(input),
    result: { content: "ok", isError: false },
    parentId: null,
    ...extra,
  }) as TranscriptItem;

function renderTranscript(items: TranscriptItem[], withCards = true) {
  const view = (
    <TranscriptView
      ref={createRef()}
      items={items}
      busy={false}
      connecting={false}
      harnessLabel="Claude Code"
      workingSince={null}
      permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
      prompt={{ onRewind: () => {} }}
    />
  );
  return render(
    withCards ? (
      <PlanCardsContext.Provider value={{ sessionId: "s1", workstreamId: "w1", revisions: planRevisions(items, {}) }}>{view}</PlanCardsContext.Provider>
    ) : (
      view
    ),
  );
}

describe("skill chip", () => {
  it("renders a Skill tool call as `<name> · skill` and reveals its instructions", () => {
    renderTranscript([user, tool("s1", "Skill", { skill: "create_plan" }, { skillBody: "Plan carefully." })]);
    const chip = screen.getByRole("button", { name: /create_plan · skill/ });
    expect(screen.queryByText("Plan carefully.")).toBeNull();
    fireEvent.click(chip);
    expect(screen.getByText("Plan carefully.")).toBeTruthy();
  });

  it("renders a skill body with no tool call the same way", () => {
    renderTranscript([user, { kind: "skill", id: "k1", name: "describe_pr", body: "b", parentId: null }]);
    expect(screen.getByText("describe_pr · skill")).toBeTruthy();
  });

  it("keeps the chip out of a collapsed tool group", () => {
    renderTranscript([
      user,
      tool("b1", "Bash", { command: "ls" }),
      tool("s1", "Skill", { skill: "create_plan" }),
      tool("b2", "Bash", { command: "pwd" }),
    ]);
    expect(screen.getByText("create_plan · skill")).toBeTruthy();
  });

  it("falls back to the tool row when the skill call failed", () => {
    renderTranscript([user, tool("s1", "Skill", { skill: "nope" }, { result: { content: "Unknown skill", isError: true } })]);
    expect(screen.queryByText("nope · skill")).toBeNull();
    expect(screen.getByText("Skill")).toBeTruthy();
  });
});

describe("plan cards", () => {
  it("replaces the last plan write of each turn with a card and marks older ones superseded", () => {
    const items = [
      user,
      tool("w1", "Write", { file_path: PLAN, content: "" }),
      tool("w2", "Edit", { file_path: PLAN, old_string: "a", new_string: "b" }),
      { ...user, id: "u2" },
      tool("w3", "Edit", { file_path: PLAN, old_string: "b", new_string: "c" }),
    ];
    renderTranscript(items);
    expect(screen.getAllByTestId("plan-card").map((c) => c.textContent)).toEqual(["w2:superseded", "w3:latest"]);
  });

  it("leaves an in-flight plan write as a tool row", () => {
    renderTranscript([user, tool("w1", "Write", { file_path: PLAN, content: "" }, { result: null })]);
    expect(screen.queryByTestId("plan-card")).toBeNull();
  });

  it("renders no cards without the session context", () => {
    renderTranscript([user, tool("w1", "Write", { file_path: PLAN, content: "" })], false);
    expect(screen.queryByTestId("plan-card")).toBeNull();
  });

  it("keeps a presented plan out of a collapsed tool group", () => {
    const items = [
      user,
      tool("b1", "Bash", { command: "ls" }),
      tool("x1", "ExitPlanMode", { plan: "# P", planFilePath: "/Users/jt/.claude/plans/p.md" }, { result: null }),
      tool("b2", "Bash", { command: "pwd" }),
    ];
    renderTranscript(items);
    expect(screen.getAllByTestId("plan-card").map((c) => c.textContent)).toEqual(["x1:latest"]);
  });

  it("renders a codex proposed plan as a card", () => {
    renderTranscript([user, tool("c1", "ProposedPlan", { plan: "# Codex" })]);
    expect(screen.getAllByTestId("plan-card").map((c) => c.textContent)).toEqual(["c1:latest"]);
  });
});
