import { describe, expect, it } from "vitest";

import {
  archiveShellState,
  clearArchiveOnRefocus,
  consumeComposerFocus,
  cycleActiveTab,
  nextShellId,
  requestComposerFocus,
  unarchiveShellState,
  useComposerFocusStore,
  type ArchivedShell,
  type ShellTab,
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

const archived = (id: number, archivedAt = 1000): ArchivedShell => ({
  shell: shell(id),
  archivedAt,
  status: "idle",
});

describe("nextShellId", () => {
  it("grows past the highest visible shell id", () => {
    expect(nextShellId([shell(1), shell(4)])).toBe(5);
  });

  it("also avoids archived ids, so an unarchived tab never collides", () => {
    // Visible tabs are 1,2; an archived tab still holds id 5. The next id must
    // clear 5 so unarchiving #5 can't clash with a freshly-minted tab.
    expect(nextShellId([shell(1), shell(2)], [archived(5)])).toBe(6);
  });
});

describe("archiveShellState", () => {
  it("moves the shell into archived (newest first) and keeps it whole", () => {
    const next = archiveShellState(mainState(1, 1, 2, 3), 2, "stopped", 1234);
    expect(next.shells.map((s) => s.id)).toEqual([1, 3]);
    expect(next.archived).toHaveLength(1);
    expect(next.archived?.[0]).toMatchObject({ archivedAt: 1234, status: "stopped" });
    expect(next.archived?.[0].shell.id).toBe(2);
  });

  it("prepends so the most recently archived agent is first", () => {
    let state = archiveShellState(mainState(1, 1, 2, 3), 2, "idle", 10);
    state = archiveShellState(state, 3, "done", 20);
    expect(state.archived?.map((a) => a.shell.id)).toEqual([3, 2]);
  });

  it("picks a neighbour when the archived shell was active", () => {
    const next = archiveShellState(mainState(2, 1, 2, 3), 2, "idle", 0);
    expect(next.activeId).toBe(3); // falls to the last remaining main tab
  });

  it("refuses to archive the main pane's last tab", () => {
    const state = mainState(1, 1);
    expect(archiveShellState(state, 1, "idle", 0)).toBe(state);
  });

  it("is a no-op for an unknown shell", () => {
    const state = mainState(1, 1, 2);
    expect(archiveShellState(state, 99, "idle", 0)).toBe(state);
  });

  it("archives a right-pane shell and collapses the split when it was the last one", () => {
    const state: WsTabState = {
      shells: [shell(1, "main"), shell(2, "right")],
      activeId: 1,
      rightSplit: true,
      rightActiveId: 2,
      activePane: "right",
    };
    const next = archiveShellState(state, 2, "idle", 0);
    expect(next.shells.map((s) => s.id)).toEqual([1]);
    expect(next.rightSplit).toBe(false);
    expect(next.activePane).toBe("main");
    expect(next.archived?.[0].shell.id).toBe(2);
  });
});

describe("unarchiveShellState", () => {
  it("restores the agent to the main strip, selects it, and drops it from archived", () => {
    const state: WsTabState = { ...mainState(1, 1), archived: [archived(5)] };
    const next = unarchiveShellState(state, 5);
    expect(next.shells.map((s) => s.id)).toEqual([1, 5]);
    expect(next.shells.find((s) => s.id === 5)?.panel).toBe("main");
    expect(next.activeId).toBe(5);
    expect(next.activePane).toBe("main");
    expect(next.archived).toHaveLength(0);
  });

  it("round-trips a shell without losing its session binding", () => {
    const bound: ShellTab = { ...shell(2), sessionId: "sess-abc", draft: "wip" };
    const start: WsTabState = { shells: [shell(1), bound], activeId: 1, activePane: "main" };
    const archivedState = archiveShellState(start, 2, "done", 0);
    const restored = unarchiveShellState(archivedState, 2);
    expect(restored.shells.find((s) => s.id === 2)).toMatchObject({
      sessionId: "sess-abc",
      draft: "wip",
    });
  });

  it("is a no-op for an id that isn't archived", () => {
    const state: WsTabState = { ...mainState(1, 1), archived: [archived(5)] };
    expect(unarchiveShellState(state, 9)).toBe(state);
  });
});

describe("clearArchiveOnRefocus", () => {
  const open = (s: WsTabState): WsTabState => ({ ...s, archiveOpen: true });

  it("closes the view when a tab is refocused (select/cycle/open)", () => {
    const prev = open(mainState(1, 1, 2));
    expect(clearArchiveOnRefocus(prev, { ...prev, activeId: 2 }).archiveOpen).toBe(false);
  });

  it("keeps the view open when a shell is removed (close/archive shrinks the strip)", () => {
    const prev = open(mainState(2, 1, 2, 3));
    const next: WsTabState = { ...prev, shells: [shell(1), shell(3)], activeId: 3 };
    expect(clearArchiveOnRefocus(prev, next).archiveOpen).toBe(true);
  });

  it("keeps the view open for a background patch that doesn't move focus", () => {
    const prev = open(mainState(1, 1, 2));
    const next: WsTabState = { ...prev, shells: [{ ...shell(1), title: "renamed" }, shell(2)] };
    expect(clearArchiveOnRefocus(prev, next).archiveOpen).toBe(true);
  });

  it("leaves a toggle that opens the view (focus unchanged) intact", () => {
    const prev = mainState(1, 1, 2);
    expect(clearArchiveOnRefocus(prev, { ...prev, archiveOpen: true }).archiveOpen).toBe(true);
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
