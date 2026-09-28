import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Session } from "@/lib/bindings";

const h = vi.hoisted(() => ({
  setSessionTitle: vi.fn(async () => ({ status: "ok", data: null })),
}));

vi.mock("../../lib/bindings", () => ({ commands: { setSessionTitle: h.setSessionTitle } }));

const { startSessionTitleSync } = await import("./sessionTitleSync");
const { takeSeenTitle } = await import("./titleSeen");
const { archiveShellState, openSessionTab, unarchiveShellState, updateWorkstreamTabs, useWorkstreamTabsStore } =
  await import("./workstreamTabs");
const { leaf } = await import("./paneTree");

const WS = "ws-sync";

const session = (id: string, title: string | null): Session => ({
  id,
  workstreamId: WS,
  stage: "general",
  harness: "claude",
  claudeSessionId: null,
  cwd: "/tmp/ws-sync",
  state: "running",
  model: null,
  costUsd: null,
  startedAt: 1000,
  endedAt: null,
  exitSummary: null,
  title,
  preview: "first prompt",
});

// One locked tab bound to `sessionId`, as a synced rename leaves it.
function seedTab(sessionId: string, title: string): void {
  useWorkstreamTabsStore.setState({
    byWs: {
      [WS]: {
        shells: [
          { id: 1, title: "Agent", mode: "general", draft: "", docs: [], attachments: [], paneId: "p1" },
          {
            id: 2,
            title,
            titleLocked: true,
            mode: "general",
            draft: "",
            docs: [],
            attachments: [],
            sessionId,
            paneId: "p1",
          },
        ],
        layout: leaf("p1"),
        panes: { p1: { activeId: 2 } },
        activePaneId: "p1",
      },
    },
  });
}

const archiveTab = () => updateWorkstreamTabs((all) => ({ ...all, [WS]: archiveShellState(all[WS], 2, "done", 1) }));
const restoreTab = () => updateWorkstreamTabs((all) => ({ ...all, [WS]: unarchiveShellState(all[WS], 2) }));
const tabFor = (sessionId: string) =>
  useWorkstreamTabsStore.getState().byWs[WS].shells.find((s) => s.sessionId === sessionId);
const rename = (sessionId: string, title: string) =>
  updateWorkstreamTabs((all) => ({
    ...all,
    [WS]: {
      ...all[WS],
      shells: all[WS].shells.map((s) => (s.sessionId === sessionId ? { ...s, title, titleLocked: true } : s)),
    },
  }));

let stop: () => void = () => {};

beforeEach(() => {
  h.setSessionTitle.mockClear();
});

afterEach(() => stop());

describe("session title sync across archive and restore", () => {
  it("pushes nothing when a tab is archived and restored from the archive", () => {
    seedTab("s-restore", "Parked name");
    stop = startSessionTitleSync();

    archiveTab();
    expect(h.setSessionTitle).not.toHaveBeenCalled();
    restoreTab();

    expect(tabFor("s-restore")?.title).toBe("Parked name");
    expect(h.setSessionTitle).not.toHaveBeenCalled();
  });

  it("adopts the server's newer title when the picker restores a parked tab, and pushes nothing", () => {
    seedTab("s-picked", "Old name");
    stop = startSessionTitleSync();
    archiveTab();

    openSessionTab(WS, session("s-picked", "Renamed elsewhere"));

    expect(tabFor("s-picked")).toMatchObject({ title: "Renamed elsewhere", titleLocked: true });
    expect(h.setSessionTitle).not.toHaveBeenCalled();
  });

  it("keeps the parked tab's own title when the server has none", () => {
    seedTab("s-untitled", "Mine");
    stop = startSessionTitleSync();
    archiveTab();

    openSessionTab(WS, session("s-untitled", null));

    expect(tabFor("s-untitled")?.title).toBe("Mine");
    expect(h.setSessionTitle).not.toHaveBeenCalled();
  });

  it("still pushes a rename of an open tab, including one just restored", () => {
    seedTab("s-rename", "Before");
    stop = startSessionTitleSync();

    rename("s-rename", "After");
    expect(h.setSessionTitle).toHaveBeenLastCalledWith("s-rename", "After");

    archiveTab();
    openSessionTab(WS, session("s-rename", "Server side"));
    rename("s-rename", "Local again");

    expect(h.setSessionTitle).toHaveBeenCalledTimes(2);
    expect(h.setSessionTitle).toHaveBeenLastCalledWith("s-rename", "Local again");
    takeSeenTitle("s-rename");
  });
});
