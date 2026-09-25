import { describe, expect, it } from "vitest";

import { leaf, leaves, type LayoutNode, type PaneId } from "./paneTree";
import {
  activeShellIn,
  archiveShellState,
  closePaneState,
  consumeComposerFocus,
  cycleActiveTab,
  isLastTab,
  mergePaneState,
  migratePersistedWorkstreamTabs,
  moveShellState,
  nextShellId,
  removeShellState,
  requestComposerFocus,
  selectLastTab,
  selectTabAt,
  splitPaneState,
  unarchiveShellState,
  useComposerFocusStore,
  visibleShells,
  type ArchivedShell,
  type ShellTab,
  type WsTabState,
} from "./workstreamTabs";

const shell = (id: number, paneId: PaneId = "p1"): ShellTab => ({
  id,
  title: `Agent ${id}`,
  mode: "general",
  draft: "",
  docs: [],
  attachments: [],
  paneId,
});

const onePane = (activeId: number, ...ids: number[]): WsTabState => ({
  shells: ids.map((id) => shell(id)),
  layout: leaf("p1"),
  panes: { p1: { activeId } },
  activePaneId: "p1",
});

const twoLeaves: LayoutNode = {
  type: "split",
  dir: "row",
  ratio: 0.5,
  a: leaf("p1"),
  b: leaf("p2"),
};

const threeLeaves: LayoutNode = {
  type: "split",
  dir: "row",
  ratio: 0.5,
  a: leaf("p1"),
  b: { type: "split", dir: "col", ratio: 0.5, a: leaf("p2"), b: leaf("p3") },
};

type PaneSpec = { active: number; ids: number[] };

const twoPanes = ({ p1, p2, focus }: { p1: PaneSpec; p2: PaneSpec; focus: PaneId }): WsTabState => ({
  shells: [...p1.ids.map((id) => shell(id, "p1")), ...p2.ids.map((id) => shell(id, "p2"))],
  layout: twoLeaves,
  panes: { p1: { activeId: p1.active }, p2: { activeId: p2.active } },
  activePaneId: focus,
});

describe("cycleActiveTab", () => {
  it("is a no-op when the focused pane has a single tab", () => {
    const state = onePane(1, 1);
    expect(cycleActiveTab(state, 1)).toBe(state);
    expect(cycleActiveTab(state, -1)).toBe(state);
  });

  it("moves to the next tab", () => {
    expect(cycleActiveTab(onePane(1, 1, 2, 3), 1).panes.p1.activeId).toBe(2);
    expect(cycleActiveTab(onePane(2, 1, 2, 3), 1).panes.p1.activeId).toBe(3);
  });

  it("moves to the previous tab", () => {
    expect(cycleActiveTab(onePane(3, 1, 2, 3), -1).panes.p1.activeId).toBe(2);
    expect(cycleActiveTab(onePane(2, 1, 2, 3), -1).panes.p1.activeId).toBe(1);
  });

  it("wraps around both ends", () => {
    expect(cycleActiveTab(onePane(3, 1, 2, 3), 1).panes.p1.activeId).toBe(1);
    expect(cycleActiveTab(onePane(1, 1, 2, 3), -1).panes.p1.activeId).toBe(3);
  });

  it("cycles in on-screen order, so it follows drag reordering", () => {
    const state = onePane(1, 3, 1, 2);
    expect(cycleActiveTab(state, 1).panes.p1.activeId).toBe(2);
    expect(cycleActiveTab(state, -1).panes.p1.activeId).toBe(3);
  });

  it("treats an unresolved active id as the first tab", () => {
    const state = onePane(99, 1, 2, 3);
    expect(cycleActiveTab(state, 1).panes.p1.activeId).toBe(2);
    expect(cycleActiveTab(state, -1).panes.p1.activeId).toBe(3);
  });

  it("cycles only the focused pane and leaves the other pane's selection alone", () => {
    const state = twoPanes({
      p1: { active: 1, ids: [1] },
      p2: { active: 2, ids: [2, 3] },
      focus: "p2",
    });
    const next = cycleActiveTab(state, 1);
    expect(next.panes.p2.activeId).toBe(3);
    expect(next.panes.p1.activeId).toBe(1);
    expect(next.activePaneId).toBe("p2");
  });

  it("is a no-op when the focused pane has fewer than two tabs", () => {
    const state = twoPanes({
      p1: { active: 1, ids: [1, 2] },
      p2: { active: 3, ids: [3] },
      focus: "p2",
    });
    expect(cycleActiveTab(state, 1)).toBe(state);
  });
});

const archived = (id: number, archivedAt = 1000): ArchivedShell => ({
  shell: shell(id),
  archivedAt,
  status: "idle",
});

describe("persisted workstream migration", () => {
  const v6 = (id: number, panel: "main" | "right") => ({ id, title: `Agent ${id}`, panel });

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

  it("turns a visible v6 split with the right pane focused into a two-pane row", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [v6(1, "main"), v6(2, "right"), v6(3, "main"), v6(4, "right")],
          activeId: 3,
          rightSplit: true,
          rightActiveId: 4,
          activePane: "right",
          archived: [{ shell: v6(5, "right"), archivedAt: 1, status: "idle" }],
        },
      },
    });

    const ws = migrated.byWs.ws;
    expect(ws.layout).toEqual(twoLeaves);
    expect(ws.panes).toEqual({ p1: { activeId: 3 }, p2: { activeId: 4 } });
    expect(ws.activePaneId).toBe("p2");
    expect(ws.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [2, "p2"],
      [3, "p1"],
      [4, "p2"],
    ]);
    expect(ws.archived?.[0].shell.paneId).toBe("p1");
    for (const key of ["activeId", "rightSplit", "rightActiveId", "activePane"]) {
      expect(ws).not.toHaveProperty(key);
    }
    for (const s of [...ws.shells, ws.archived![0].shell]) expect(s).not.toHaveProperty("panel");
  });

  it("shows a hidden v6 right pane's tabs in the single pane, in array order", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [v6(1, "main"), v6(2, "right"), v6(3, "main"), v6(4, "right")],
          activeId: 3,
          rightSplit: false,
          rightActiveId: 2,
          activePane: "main",
        },
      },
    });

    const ws = migrated.byWs.ws;
    expect(ws.layout).toEqual(leaf("p1"));
    expect(ws.panes).toEqual({ p1: { activeId: 3 } });
    expect(ws.activePaneId).toBe("p1");
    expect(ws.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [2, "p1"],
      [3, "p1"],
      [4, "p1"],
    ]);
  });

  it("gives a v6 workstream with no split keys at all one pane", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: { ws: { shells: [v6(1, "main"), v6(2, "main")], activeId: 2 } },
    });

    const ws = migrated.byWs.ws;
    expect(ws.layout).toEqual(leaf("p1"));
    expect(ws.panes).toEqual({ p1: { activeId: 2 } });
    expect(ws.activePaneId).toBe("p1");
  });

  it("gives a v6 rightSplit with no right shells one pane", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [v6(1, "main"), v6(2, "main")],
          activeId: 1,
          rightSplit: true,
          activePane: "right",
        },
      },
    });

    expect(migrated.byWs.ws.layout).toEqual(leaf("p1"));
    expect(migrated.byWs.ws.activePaneId).toBe("p1");
  });

  it("passes an already-v7 store through with its layout reference intact, idempotently", () => {
    const state = twoPanes({
      p1: { active: 1, ids: [1, 2] },
      p2: { active: 3, ids: [3] },
      focus: "p2",
    });
    const migrated = migratePersistedWorkstreamTabs({ byWs: { ws: state } });

    expect(migrated.byWs.ws.layout).toBe(twoLeaves);
    expect(migrated.byWs.ws.panes).toEqual({ p1: { activeId: 1 }, p2: { activeId: 3 } });
    expect(migrated.byWs.ws.activePaneId).toBe("p2");
    expect(migratePersistedWorkstreamTabs(JSON.parse(JSON.stringify(migrated)))).toEqual(migrated);
  });

  it("reconciles a store re-upgraded after rollback: orphans join the first pane, empty panes go", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [{ ...shell(1, "p1"), panel: "main" }, v6(9, "main")],
          layout: twoLeaves,
          panes: { p1: { activeId: 1 }, p2: { activeId: 2 } },
          activePaneId: "p2",
          activeId: 9,
          archived: [{ shell: shell(5, "p2"), archivedAt: 1, status: "idle" }],
        },
      },
    });

    const ws = migrated.byWs.ws;
    expect(ws.layout).toEqual(leaf("p1"));
    expect(ws.panes).toEqual({ p1: { activeId: 1 } });
    expect(ws.activePaneId).toBe("p1");
    expect(ws.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [9, "p1"],
    ]);
    expect(ws.archived?.[0].shell.paneId).toBe("p1");
    expect(ws).not.toHaveProperty("activeId");
    for (const s of ws.shells) expect(s).not.toHaveProperty("panel");
  });

  it("repairs stale and missing per-pane activeIds, and leaves a good one alone", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [shell(1, "p1"), shell(2, "p1"), shell(3, "p2"), shell(4, "p3"), shell(5, "p3")],
          layout: threeLeaves,
          panes: { p1: { activeId: 9 }, p3: { activeId: 5 } },
          activePaneId: "p1",
        },
      },
    });

    const ws = migrated.byWs.ws;
    expect(ws.panes).toEqual({ p1: { activeId: 1 }, p2: { activeId: 3 }, p3: { activeId: 5 } });
    for (const id of leaves(ws.layout)) {
      expect(ws.panes[id].activeId).toBe(activeShellIn(ws, id)?.id);
    }
    expect(migratePersistedWorkstreamTabs(JSON.parse(JSON.stringify(migrated)))).toEqual(migrated);
  });

  it("migrates a v6 rightActiveId that names a main shell without mounting one shell twice", () => {
    const migrated = migratePersistedWorkstreamTabs({
      byWs: {
        ws: {
          shells: [v6(1, "main"), v6(2, "main"), v6(3, "right")],
          activeId: 2,
          rightSplit: true,
          rightActiveId: 1,
          activePane: "main",
        },
      },
    });

    const ws = migrated.byWs.ws;
    expect(activeShellIn(ws, "p1")?.id).toBe(2);
    expect(activeShellIn(ws, "p2")?.id).toBe(3);
    expect(visibleShells(ws).map((s) => s.id)).toEqual([2, 3]);
  });

  it("drops the retired archiveOpen flag", () => {
    const migrated = migratePersistedWorkstreamTabs({ byWs: { ws: { ...onePane(1, 1), archiveOpen: true } } });
    expect(migrated.byWs.ws).not.toHaveProperty("archiveOpen");
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

describe("activeShellIn and isLastTab", () => {
  it("ignores an activeId that names a shell in another pane and falls back to the pane's first tab", () => {
    const state = twoPanes({
      p1: { active: 3, ids: [1, 2] },
      p2: { active: 3, ids: [3] },
      focus: "p1",
    });
    expect(activeShellIn(state, "p1")?.id).toBe(1);
    expect(activeShellIn(state, "p2")?.id).toBe(3);
  });

  it("is the last tab only when it is the one tab of the one pane", () => {
    expect(isLastTab(onePane(1, 1), 1)).toBe(true);
    expect(isLastTab(onePane(1, 1, 2), 1)).toBe(false);
    expect(isLastTab(onePane(1, 1), 99)).toBe(false);
    const split = twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: 2, ids: [2] }, focus: "p1" });
    expect(isLastTab(split, 1)).toBe(false);
    expect(isLastTab(split, 2)).toBe(false);
  });
});

describe("removeShellState", () => {
  it("selects the tab to the right of the closed one", () => {
    expect(removeShellState(onePane(2, 1, 2, 3, 4), 2).panes.p1.activeId).toBe(3);
    expect(removeShellState(onePane(1, 1, 2, 3, 4), 1).panes.p1.activeId).toBe(2);
  });

  it("falls back to the new rightmost tab when the closed one was last", () => {
    expect(removeShellState(onePane(4, 1, 2, 3, 4), 4).panes.p1.activeId).toBe(3);
  });

  it("leaves the selection alone when the closed tab was not active", () => {
    expect(removeShellState(onePane(4, 1, 2, 3, 4), 2).panes.p1.activeId).toBe(4);
    expect(removeShellState(onePane(1, 1, 2, 3, 4), 3).panes.p1.activeId).toBe(1);
  });

  it("respects on-screen order, not id order, after a drag-reorder", () => {
    expect(removeShellState(onePane(3, 3, 1, 2), 3).panes.p1.activeId).toBe(1);
  });

  it("refuses to remove the last tab of the last pane", () => {
    const state = onePane(1, 1);
    expect(removeShellState(state, 1)).toBe(state);
  });

  it("is a no-op for an unknown shell", () => {
    const state = onePane(1, 1, 2);
    expect(removeShellState(state, 99)).toBe(state);
  });

  const splitState = (p2Active: number): WsTabState =>
    twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: p2Active, ids: [2, 3, 4] }, focus: "p2" });

  it("applies the same right-hand rule inside the second pane", () => {
    expect(removeShellState(splitState(2), 2).panes.p2.activeId).toBe(3);
    expect(removeShellState(splitState(4), 4).panes.p2.activeId).toBe(3);
    expect(removeShellState(splitState(2), 2).panes.p1.activeId).toBe(1);
  });

  it("removes the emptied pane, drops its panes entry, and focuses the heir", () => {
    const state = twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: 2, ids: [2] }, focus: "p2" });
    const next = removeShellState(state, 2);
    expect(next.layout).toEqual(leaf("p1"));
    expect(next.panes).not.toHaveProperty("p2");
    expect(next.activePaneId).toBe("p1");
  });

  it("no longer protects the first pane's last tab while another pane exists", () => {
    const state = twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: 2, ids: [2, 3] }, focus: "p1" });
    const next = removeShellState(state, 1);
    expect(next.layout).toEqual(leaf("p2"));
    expect(next.panes).toEqual({ p2: { activeId: 2 } });
    expect(next.activePaneId).toBe("p2");
  });

  it("focuses the heir when a focused middle pane of three loses its last tab", () => {
    const state: WsTabState = {
      shells: [shell(1, "p1"), shell(2, "p2"), shell(3, "p3"), shell(4, "p3")],
      layout: threeLeaves,
      panes: { p1: { activeId: 1 }, p2: { activeId: 2 }, p3: { activeId: 4 } },
      activePaneId: "p2",
    };
    const next = removeShellState(state, 2);
    expect(next.layout).toEqual({ ...twoLeaves, b: leaf("p3") });
    expect(next.panes).toEqual({ p1: { activeId: 1 }, p3: { activeId: 4 } });
    expect(next.activePaneId).toBe("p3");
  });
});

describe("selectTabAt", () => {
  it("selects the n-th tab of the focused pane, 0-based", () => {
    expect(selectTabAt(onePane(1, 1, 2, 3), 0).panes.p1.activeId).toBe(1);
    expect(selectTabAt(onePane(1, 1, 2, 3), 2).panes.p1.activeId).toBe(3);
  });

  it("is a no-op when the pane has no such tab", () => {
    const state = onePane(1, 1, 2, 3);
    expect(selectTabAt(state, 3)).toBe(state);
  });

  it("selects within the focused pane and focuses it", () => {
    const state = twoPanes({
      p1: { active: 1, ids: [1] },
      p2: { active: 3, ids: [2, 3] },
      focus: "p2",
    });
    const next = selectTabAt(state, 0);
    expect(next.panes.p2.activeId).toBe(2);
    expect(next.activePaneId).toBe("p2");
    expect(next.panes.p1.activeId).toBe(1);
  });
});

describe("selectLastTab", () => {
  it("selects the rightmost tab, however many there are", () => {
    expect(selectLastTab(onePane(1, 1, 2)).panes.p1.activeId).toBe(2);
    expect(selectLastTab(onePane(1, 1, 2, 3, 4, 5)).panes.p1.activeId).toBe(5);
  });

  it("is a no-op when it is already the last tab", () => {
    expect(selectLastTab(onePane(3, 1, 2, 3)).panes.p1.activeId).toBe(3);
  });
});

describe("moveShellState", () => {
  it("collapses the source pane it empties and ignores an unknown or unchanged target", () => {
    const state = twoPanes({ p1: { active: 1, ids: [1, 2] }, p2: { active: 3, ids: [3] }, focus: "p2" });
    const next = moveShellState(state, 3, "p1");
    expect(next.layout).toEqual(leaf("p1"));
    expect(next.panes).toEqual({ p1: { activeId: 3 } });
    expect(next.activePaneId).toBe("p1");
    expect(next.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [2, "p1"],
      [3, "p1"],
    ]);
    expect(moveShellState(state, 1, "p9")).toBe(state);
    expect(moveShellState(state, 1, "p1")).toBe(state);
  });

  it("hands the source pane's selection to the right-hand neighbour when its active tab leaves", () => {
    const state = twoPanes({ p1: { active: 2, ids: [1, 2, 3] }, p2: { active: 4, ids: [4] }, focus: "p1" });
    const next = moveShellState(state, 2, "p2");
    expect(next.panes).toEqual({ p1: { activeId: 3 }, p2: { activeId: 2 } });
    expect(next.activePaneId).toBe("p2");
    expect(next.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [2, "p2"],
      [3, "p1"],
      [4, "p2"],
    ]);
  });
});

describe("splitPaneState", () => {
  it("does nothing for a single-tab pane", () => {
    const state = onePane(1, 1);
    expect(splitPaneState(state, "p1", "row", "after", 1)).toBe(state);
  });

  it("moves the tab into a new pane after the target and keeps the target's selection", () => {
    const next = splitPaneState(onePane(1, 1, 2, 3), "p1", "row", "after", 3);
    expect(next.layout).toEqual(twoLeaves);
    expect(next.panes).toEqual({ p1: { activeId: 1 }, p2: { activeId: 3 } });
    expect(next.activePaneId).toBe("p2");
    expect(next.shells.find((s) => s.id === 3)?.paneId).toBe("p2");
  });
});

describe("mergePaneState", () => {
  it("appends merged tabs after the heir's and leaves the heir's active tab unchanged", () => {
    const state: WsTabState = {
      ...twoPanes({ p1: { active: 3, ids: [] }, p2: { active: 4, ids: [] }, focus: "p2" }),
      shells: [shell(1, "p1"), shell(2, "p2"), shell(3, "p1"), shell(4, "p2")],
    };
    const next = mergePaneState(state, "p2");
    expect(next.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [3, "p1"],
      [2, "p1"],
      [4, "p1"],
    ]);
    expect(next.layout).toEqual(leaf("p1"));
    expect(next.panes).toEqual({ p1: { activeId: 3 } });
    expect(next.activePaneId).toBe("p1");
  });

  it("keeps every shell and puts it in the heir", () => {
    const state: WsTabState = {
      shells: [shell(1, "p1"), shell(2, "p2"), shell(3, "p3"), shell(4, "p3")],
      layout: threeLeaves,
      panes: { p1: { activeId: 1 }, p2: { activeId: 2 }, p3: { activeId: 3 } },
      activePaneId: "p1",
    };
    const next = mergePaneState(state, "p3");
    expect(next.shells.map((s) => [s.id, s.paneId])).toEqual([
      [1, "p1"],
      [2, "p2"],
      [3, "p2"],
      [4, "p2"],
    ]);
    expect(next.layout).toEqual(twoLeaves);
    expect(next.activePaneId).toBe("p1");
  });

  it("refuses on one leaf", () => {
    const state = onePane(1, 1, 2);
    expect(mergePaneState(state, "p1")).toBe(state);
  });
});

describe("closePaneState", () => {
  it("refuses on one leaf", () => {
    const state = onePane(1, 1, 2);
    expect(closePaneState(state, "p1")).toBe(state);
  });

  it("drops the pane with its shells and focuses the heir", () => {
    const state = twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: 2, ids: [2, 3] }, focus: "p2" });
    const next = closePaneState(state, "p2");
    expect(next.shells.map((s) => s.id)).toEqual([1]);
    expect(next.layout).toEqual(leaf("p1"));
    expect(next.panes).toEqual({ p1: { activeId: 1 } });
    expect(next.activePaneId).toBe("p1");
  });
});

describe("archiveShellState", () => {
  it("moves the shell into archived (newest first) and keeps it whole", () => {
    const next = archiveShellState(onePane(1, 1, 2, 3), 2, "stopped", 1234);
    expect(next.shells.map((s) => s.id)).toEqual([1, 3]);
    expect(next.archived).toHaveLength(1);
    expect(next.archived?.[0]).toMatchObject({ archivedAt: 1234, status: "stopped" });
    expect(next.archived?.[0].shell.id).toBe(2);
  });

  it("prepends so the most recently archived agent is first", () => {
    let state = archiveShellState(onePane(1, 1, 2, 3), 2, "idle", 10);
    state = archiveShellState(state, 3, "done", 20);
    expect(state.archived?.map((a) => a.shell.id)).toEqual([3, 2]);
  });

  it("picks the right-hand neighbour when the archived shell was active", () => {
    const next = archiveShellState(onePane(2, 1, 2, 3), 2, "idle", 0);
    expect(next.panes.p1.activeId).toBe(3);
  });

  it("refuses to archive the last tab of the last pane", () => {
    const state = onePane(1, 1);
    expect(archiveShellState(state, 1, "idle", 0)).toBe(state);
  });

  it("is a no-op for an unknown shell", () => {
    const state = onePane(1, 1, 2);
    expect(archiveShellState(state, 99, "idle", 0)).toBe(state);
  });

  it("archives a second-pane shell and removes the emptied pane, focusing the heir", () => {
    const state = twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: 2, ids: [2] }, focus: "p2" });
    const next = archiveShellState(state, 2, "idle", 0);
    expect(next.shells.map((s) => s.id)).toEqual([1]);
    expect(next.layout).toEqual(leaf("p1"));
    expect(next.panes).not.toHaveProperty("p2");
    expect(next.activePaneId).toBe("p1");
    expect(next.archived?.[0].shell.id).toBe(2);
  });
});

describe("unarchiveShellState", () => {
  it("restores the agent to the focused pane, selects it, and drops it from archived", () => {
    const state: WsTabState = {
      ...onePane(1, 1),
      archived: [{ ...archived(5), shell: shell(5, "p2") }],
    };
    const next = unarchiveShellState(state, 5);
    expect(next.shells.map((s) => s.id)).toEqual([1, 5]);
    expect(next.shells.find((s) => s.id === 5)?.paneId).toBe("p1");
    expect(next.panes.p1.activeId).toBe(5);
    expect(next.activePaneId).toBe("p1");
    expect(next.archived).toHaveLength(0);
  });

  it("restores to the focused pane when it isn't the first", () => {
    const state: WsTabState = {
      ...twoPanes({ p1: { active: 1, ids: [1] }, p2: { active: 2, ids: [2] }, focus: "p2" }),
      archived: [{ ...archived(5), shell: shell(5, "p1") }],
    };
    const next = unarchiveShellState(state, 5);
    expect(next.shells.find((s) => s.id === 5)?.paneId).toBe("p2");
    expect(next.panes.p2.activeId).toBe(5);
    expect(next.activePaneId).toBe("p2");
  });

  it("round-trips a shell without losing its session binding", () => {
    const bound: ShellTab = { ...shell(2), sessionId: "sess-abc", draft: "wip" };
    const start: WsTabState = { ...onePane(1), shells: [shell(1), bound] };
    const archivedState = archiveShellState(start, 2, "done", 0);
    const restored = unarchiveShellState(archivedState, 2);
    expect(restored.shells.find((s) => s.id === 2)).toMatchObject({
      sessionId: "sess-abc",
      draft: "wip",
    });
  });

  it("is a no-op for an id that isn't archived", () => {
    const state: WsTabState = { ...onePane(1, 1), archived: [archived(5)] };
    expect(unarchiveShellState(state, 9)).toBe(state);
  });
});

describe("composer focus signal", () => {
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
