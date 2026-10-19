// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { WorkstreamView } from "@/lib/bindings";
import { closeGoalSheet, useGoalSheet } from "@/lib/goalSheet";

import { FixUntilMetButton } from "./goalActions";
import type { ValidateFlow } from "./validateFlow";

const PLAN = "/t/plans/widget.md";
const FINDINGS = ["render() ignores the theme", "Failed check: cargo test -p widget"];
const VIEW = { item: { id: "w1" } } as WorkstreamView;

const flow = (over: Partial<ValidateFlow> = {}) =>
  ({ findings: FINDINGS, planPath: PLAN, implement: { id: "impl1" }, busy: null, ...over }) as ValidateFlow;

afterEach(() => {
  cleanup();
  closeGoalSheet();
});

describe("FixUntilMetButton", () => {
  it("opens the sheet with the findings, the plan path and the implement session", () => {
    render(<FixUntilMetButton flow={flow()} view={VIEW} />);
    fireEvent.click(screen.getByRole("button", { name: "Fix until met" }));
    expect(useGoalSheet.getState().request).toEqual({
      mode: "start",
      workstreamId: "w1",
      source: "validation",
      findings: FINDINGS,
      planPath: PLAN,
      target: { sessionId: "impl1" },
    });
  });

  it("is disabled without an implement session to target", () => {
    render(<FixUntilMetButton flow={flow({ implement: null })} view={VIEW} />);
    const button = screen.getByRole("button", { name: "Fix until met" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(useGoalSheet.getState().request).toBeNull();
  });
});
