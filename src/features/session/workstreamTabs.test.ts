import { describe, expect, it } from "vitest";

import type { ShellTab } from "../../routes/ProjectPage";
import {
  consumeComposerFocus,
  cycleActiveTab,
  requestComposerFocus,
  useComposerFocusStore,
  type WsTabState,
} from "./workstreamTabs";

const shell = (id: number, panel: "main" | "right" = "main"): ShellTab => ({
  id,
  title: `Agent ${id}`,
  mode: "general",
  draft: "",
  docs: [],
  attachments: [],
  panel,
});

const mainState = (activeId: number, ...ids: number[]): WsTabState => ({
  shells: ids.map((id) => shell(id, "main")),
  activeId,
  activePane: "main",
});

describe("cycleActiveTab", () => {
  it("is a no-op when the focused pane has a single tab", () => {
    const state = mainState(1, 1);
    expect(cycleActiveTab(state, 1)).toBe(state);
    expect(cycleActiveTab(state, -1)).toBe(state);
  });

  it("moves to the next tab", () => {
    expect(cycleActiveTab(mainState(1, 1, 2, 3), 1).activeId).toBe(2);
    expect(cycleActiveTab(mainState(2, 1, 2, 3), 1).activeId).toBe(3);
  });

  it("moves to the previous tab", () => {
    expect(cycleActiveTab(mainState(3, 1, 2, 3), -1).activeId).toBe(2);
    expect(cycleActiveTab(mainState(2, 1, 2, 3), -1).activeId).toBe(1);
  });

  it("wraps around both ends", () => {
    expect(cycleActiveTab(mainState(3, 1, 2, 3), 1).activeId).toBe(1); // last → first
    expect(cycleActiveTab(mainState(1, 1, 2, 3), -1).activeId).toBe(3); // first → last
  });

  it("cycles in on-screen order, so it follows drag reordering", () => {
    // shells stored as [3, 1, 2] (e.g. after a drag); active is the middle one.
    const state: WsTabState = {
      shells: [shell(3), shell(1), shell(2)],
      activeId: 1,
      activePane: "main",
    };
    expect(cycleActiveTab(state, 1).activeId).toBe(2);
    expect(cycleActiveTab(state, -1).activeId).toBe(3);
  });

  it("treats an unresolved active id as the first tab", () => {
    const state = mainState(99, 1, 2, 3); // 99 isn't a real tab
    expect(cycleActiveTab(state, 1).activeId).toBe(2); // first → next
    expect(cycleActiveTab(state, -1).activeId).toBe(3); // first → previous wraps to last
  });

  it("cycles only the focused right pane and leaves the main selection alone", () => {
    const state: WsTabState = {
      shells: [shell(1, "main"), shell(2, "right"), shell(3, "right")],
      activeId: 1,
      rightSplit: true,
      rightActiveId: 2,
      activePane: "right",
    };
    const next = cycleActiveTab(state, 1);
    expect(next.rightActiveId).toBe(3);
    expect(next.activeId).toBe(1); // main pane untouched
    expect(next.activePane).toBe("right");
  });

  it("is a no-op when the focused pane has fewer than two tabs", () => {
    // Focus is on the right pane, but it holds a single tab.
    const state: WsTabState = {
      shells: [shell(1, "main"), shell(2, "main"), shell(3, "right")],
      activeId: 1,
      rightSplit: true,
      rightActiveId: 3,
      activePane: "right",
    };
    expect(cycleActiveTab(state, 1)).toBe(state);
  });
});

describe("composer focus signal", () => {
  it("starts with no pending request", () => {
    useComposerFocusStore.setState({ pendingShellId: null });
    expect(useComposerFocusStore.getState().pendingShellId).toBeNull();
  });

  it("records a pending request for the target shell", () => {
    requestComposerFocus(7);
    expect(useComposerFocusStore.getState().pendingShellId).toBe(7);
  });

  it("only the targeted shell consumes it, and exactly once", () => {
    requestComposerFocus(7);
    expect(consumeComposerFocus(3)).toBe(false); // wrong shell — left pending
    expect(useComposerFocusStore.getState().pendingShellId).toBe(7);
    expect(consumeComposerFocus(7)).toBe(true); // target claims it
    expect(useComposerFocusStore.getState().pendingShellId).toBeNull();
    expect(consumeComposerFocus(7)).toBe(false); // already consumed — never re-fires
  });
});
