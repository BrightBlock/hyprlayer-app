import { describe, expect, it } from "vitest";

import {
  archiveShellState,
  clearArchiveOnRefocus,
  consumeComposerFocus,
  cycleActiveTab,
  migratePersistedWorkstreamTabs,
  nextShellId,
  removeShellState,
  requestComposerFocus,
  selectLastTab,
  selectTabAt,
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
    expect(cycleActiveTab(mainState(3, 1, 2, 3), 1).activeId).toBe(1);
    expect(cycleActiveTab(mainState(1, 1, 2, 3), -1).activeId).toBe(3);
  });

  it("cycles in on-screen order, so it follows drag reordering", () => {
    const state: WsTabState = {
      shells: [shell(3), shell(1), shell(2)],
      activeId: 1,
      activePane: "main",
    };
    expect(cycleActiveTab(state, 1).activeId).toBe(2);
    expect(cycleActiveTab(state, -1).activeId).toBe(3);
  });

  it("treats an unresolved active id as the first tab", () => {
    const state = mainState(99, 1, 2, 3);
    expect(cycleActiveTab(state, 1).activeId).toBe(2);
    expect(cycleActiveTab(state, -1).activeId).toBe(3);
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
    expect(next.activeId).toBe(1);
    expect(next.activePane).toBe("right");
  });

  it("is a no-op when the focused pane has fewer than two tabs", () => {
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

describe("persisted workstream migration", () => {
  it("clears legacy Copilot picks from active and archived shells case-insensitively", () => {
    const current = { ...shell(1), opencodeModel: "GitHub-Copilot/gpt-5" };
    const supported = { ...shell(2), opencodeModel: "openai/gpt-5" };
    const oldArchived = {
      ...archived(3),
      shell: { ...shell(3), opencodeModel: "GITHUB-COPILOT/claude-sonnet" },
    };

    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [current, supported],
          activeId: 1,
          archived: [oldArchived],
        },
      },
    });

    expect(migrated.byWs.ws.shells[0].opencodeModel).toBeUndefined();
    expect(migrated.byWs.ws.shells[1].opencodeModel).toBe("openai/gpt-5");
    expect(migrated.byWs.ws.archived?.[0].shell.opencodeModel).toBeUndefined();
  });

  it("drops malformed entries instead of throwing away the whole store", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [shell(1), null],
          activeId: 1,
          archived: [archived(2), { archivedAt: 5, status: "idle" }, null],
        },
        broken: null,
      },
    });

    expect(migrated.byWs.ws.shells.map((s) => s.id)).toEqual([1]);
    expect(migrated.byWs.ws.archived?.map((e) => e.shell.id)).toEqual([2]);
  });

  it("backfills missing per-shell defaults on both live and archived shells", () => {
    const bare = { id: 7, title: "t", activityAt: 0 } as unknown as ReturnType<typeof shell>;
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: { shells: [bare], activeId: 7, archived: [{ ...archived(8), shell: bare }] },
      },
    });

    for (const s of [migrated.byWs.ws.shells[0], migrated.byWs.ws.archived![0].shell]) {
      expect(s.docs).toEqual([]);
      expect(s.attachments).toEqual([]);
      expect(s.dismissedSuggestions).toEqual([]);
      expect(s.titleLocked).toBe(true);
    }
  });
});

describe("nextShellId", () => {
  it("grows past the highest visible shell id", () => {
    expect(nextShellId([shell(1), shell(4)])).toBe(5);
  });

  it("also avoids archived ids, so an unarchived tab never collides", () => {
    expect(nextShellId([shell(1), shell(2)], [archived(5)])).toBe(6);
  });
});

describe("removeShellState", () => {
  it("selects the tab to the right of the closed one", () => {
    expect(removeShellState(mainState(2, 1, 2, 3, 4), 2).activeId).toBe(3);
    expect(removeShellState(mainState(1, 1, 2, 3, 4), 1).activeId).toBe(2);
  });

  it("falls back to the new rightmost tab when the closed one was last", () => {
    expect(removeShellState(mainState(4, 1, 2, 3, 4), 4).activeId).toBe(3);
  });

  it("leaves the selection alone when the closed tab was not active", () => {
    expect(removeShellState(mainState(4, 1, 2, 3, 4), 2).activeId).toBe(4);
    expect(removeShellState(mainState(1, 1, 2, 3, 4), 3).activeId).toBe(1);
  });

  it("respects on-screen order, not id order, after a drag-reorder", () => {
    const state: WsTabState = {
      shells: [shell(3), shell(1), shell(2)],
      activeId: 3,
      activePane: "main",
    };
    expect(removeShellState(state, 3).activeId).toBe(1);
  });

  it("refuses to remove the main pane's last tab", () => {
    const state = mainState(1, 1);
    expect(removeShellState(state, 1)).toBe(state);
  });

  it("is a no-op for an unknown shell", () => {
    const state = mainState(1, 1, 2);
    expect(removeShellState(state, 99)).toBe(state);
  });

  const splitState = (rightActiveId: number): WsTabState => ({
    shells: [shell(1, "main"), shell(2, "right"), shell(3, "right"), shell(4, "right")],
    activeId: 1,
    rightSplit: true,
    rightActiveId,
    activePane: "right",
  });

  it("applies the same right-hand rule inside the right pane", () => {
    expect(removeShellState(splitState(2), 2).rightActiveId).toBe(3);
    expect(removeShellState(splitState(4), 4).rightActiveId).toBe(3);
    expect(removeShellState(splitState(2), 2).activeId).toBe(1);
  });

  it("collapses the split when the right pane's last tab goes", () => {
    const state: WsTabState = {
      shells: [shell(1, "main"), shell(2, "right")],
      activeId: 1,
      rightSplit: true,
      rightActiveId: 2,
      activePane: "right",
    };
    const next = removeShellState(state, 2);
    expect(next.rightActiveId).toBeUndefined();
    expect(next.rightSplit).toBe(false);
    expect(next.activePane).toBe("main");
  });
});

describe("selectTabAt", () => {
  it("selects the n-th tab of the focused pane, 0-based", () => {
    expect(selectTabAt(mainState(1, 1, 2, 3), 0).activeId).toBe(1);
    expect(selectTabAt(mainState(1, 1, 2, 3), 2).activeId).toBe(3);
  });

  it("is a no-op when the pane has no such tab", () => {
    const state = mainState(1, 1, 2, 3);
    expect(selectTabAt(state, 3)).toBe(state);
  });

  it("selects within the focused pane and focuses it", () => {
    const state: WsTabState = {
      shells: [shell(1, "main"), shell(2, "right"), shell(3, "right")],
      activeId: 1,
      rightSplit: true,
      rightActiveId: 3,
      activePane: "right",
    };
    const next = selectTabAt(state, 0);
    expect(next.rightActiveId).toBe(2);
    expect(next.activePane).toBe("right");
    expect(next.activeId).toBe(1);
  });
});

describe("selectLastTab", () => {
  it("selects the rightmost tab, however many there are", () => {
    expect(selectLastTab(mainState(1, 1, 2)).activeId).toBe(2);
    expect(selectLastTab(mainState(1, 1, 2, 3, 4, 5)).activeId).toBe(5);
  });

  it("is a no-op when it is already the last tab", () => {
    expect(selectLastTab(mainState(3, 1, 2, 3)).activeId).toBe(3);
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

  it("picks the right-hand neighbour when the archived shell was active", () => {
    const next = archiveShellState(mainState(2, 1, 2, 3), 2, "idle", 0);
    expect(next.activeId).toBe(3);
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
    expect(consumeComposerFocus(3)).toBe(false);
    expect(useComposerFocusStore.getState().pendingShellId).toBe(7);
    expect(consumeComposerFocus(7)).toBe(true);
    expect(useComposerFocusStore.getState().pendingShellId).toBeNull();
    expect(consumeComposerFocus(7)).toBe(false);
  });
});
