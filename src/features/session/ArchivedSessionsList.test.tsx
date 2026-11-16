// @vitest-environment jsdom
// Failure modes, each covered below:
//  1. Restore lands in the wrong pane (not the focused one) or leaves the agent unselected.
//  2. Restore leaves the entry in `archived`, duplicating the agent.
//  3. Restore leaves the popover open or requests no composer focus.
//  4. Delete runs without confirmation, or Cancel deletes anyway.
//  5. Delete leaks the transcript: no `dropSession` for the entry's `sessionId`.
//  6. Deleting a PTY agent with no `sessionId` calls `dropSession(undefined)`.
//  7. After a delete, focus falls to <body> instead of the next row, or the filter when empty.
//  8. The filter is case-sensitive or ignores the model label.
//  9. Rows follow array order instead of `archivedAt` descending.
// 10. The popover opens while disconnected, stacking over the HostsDialog.
// 11. The popover stays open across a workstream change, showing another workstream's archive.
// 12. The Archived switch reads "0".
// 13. A workstream with no tab state crashes instead of showing the empty state.
// 14. Enter or Delete on a focused row button is also handled by the row.
// 15. ⌘⇧A does not toggle the popover.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runCommand } from "@/lib/keybindings/registry";

const h = vi.hoisted(() => ({ useConnectionStatus: vi.fn(), dropSession: vi.fn() }));
vi.mock("@/lib/queries", () => ({
  useConnectionStatus: h.useConnectionStatus,
  useClaudeProbe: () => ({ data: undefined }),
  useWorkstreamSessions: () => ({ data: undefined }),
  useActiveSessions: () => ({ data: undefined }),
  useBootstrap: () => ({ data: undefined }),
  usePtyAgents: () => ({ data: undefined }),
}));
vi.mock("./sessionStore", () => ({ dropSession: h.dropSession }));

import { AgentSessionsButton } from "./AgentSessionsButton";
import { closeAgentSessions, useAgentSessions } from "./agentSessionsStore";
import { leaf } from "./paneTree";
import {
  useComposerFocusStore,
  useWorkstreamTabsStore,
  type ShellTab,
} from "./workstreamTabs";

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const NOW = Date.UTC(2026, 8, 25, 12, 0, 0);

const shell = (id: number, over: Partial<ShellTab> = {}): ShellTab => ({
  id,
  title: `Agent ${id}`,
  mode: "general",
  draft: "",
  docs: [],
  attachments: [],
  paneId: "p1",
  ...over,
});

function seed() {
  useWorkstreamTabsStore.setState({
    byWs: {
      w1: {
        shells: [shell(1), shell(2, { paneId: "p2" })],
        layout: { type: "split", dir: "row", ratio: 0.5, a: leaf("p1"), b: leaf("p2") },
        panes: { p1: { activeId: 1 }, p2: { activeId: 2 } },
        activePaneId: "p2",
        archived: [
          {
            shell: shell(5, { title: "Refactor auth", model: "sonnet" }),
            archivedAt: NOW - 2 * HOUR,
            status: "done",
          },
          {
            shell: shell(6, {
              title: "Spike codex",
              harness: "codex",
              codexModel: "gpt-5-codex",
              sessionId: "sess-6",
            }),
            archivedAt: NOW - 3 * DAY,
            status: "stopped",
          },
          {
            shell: shell(7, { title: "Old terminal agent", ptyAgent: { engineId: "gemini" } }),
            archivedAt: NOW - HOUR,
            status: "idle",
          },
        ],
      },
    },
  });
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  h.useConnectionStatus.mockReturnValue({ data: { state: "connected" } });
  seed();
});

afterEach(() => {
  act(() => closeAgentSessions());
  cleanup();
  useWorkstreamTabsStore.setState({ byWs: {} });
  useComposerFocusStore.setState({ pendingShellId: null });
  vi.useRealTimers();
  vi.clearAllMocks();
});

const trigger = () => screen.getByRole("button", { name: "Agent sessions" });
const popover = () => screen.queryByRole("dialog", { name: "Agent sessions" });
const rowFor = (title: string) => screen.getByRole("listitem", { name: new RegExp(`^${title},`) });
const names = () => screen.queryAllByRole("listitem").map((li) => li.getAttribute("aria-label")?.split(",")[0]);
const filterBox = () => screen.getByRole("textbox", { name: "Filter archived sessions" });
const archivedIds = (ws = "w1") =>
  (useWorkstreamTabsStore.getState().byWs[ws]?.archived ?? []).map((a) => a.shell.id);

function openPopover(workstreamId = "w1") {
  render(<AgentSessionsButton workstreamId={workstreamId} />);
  fireEvent.click(trigger());
  fireEvent.click(screen.getByRole("radio", { name: /^Archived/ }));
  const dialog = popover();
  expect(dialog).not.toBeNull();
  return dialog!;
}

function askDelete(title: string) {
  fireEvent.click(within(rowFor(title)).getByRole("button", { name: `Delete ${title}` }));
  return screen.getByRole("dialog", { name: `Delete ${title}?` });
}

describe("ArchivedSessionsList", () => {
  it("counts only a non-empty archive, and a workstream with no tabs says so", () => {
    openPopover();
    expect(screen.getByRole("radio", { name: "Archived 3" })).toBeTruthy();
    act(() => closeAgentSessions());
    cleanup();

    openPopover("w2");
    expect(screen.getByRole("radio", { name: "Archived" })).toBeTruthy();
    expect(screen.getByText("No archived sessions.")).toBeTruthy();
    expect(names()).toEqual([]);
  });

  it("opens as a labelled dialog, newest first, with model and age on each row", () => {
    const dialog = openPopover();
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
    expect(within(dialog).getByRole("radio", { name: "Archived 3" })).toBeTruthy();
    expect(names()).toEqual(["Old terminal agent", "Refactor auth", "Spike codex"]);
    expect(within(rowFor("Old terminal agent")).getByText("Gemini · 1h")).toBeTruthy();
    expect(within(rowFor("Refactor auth")).getByText("Sonnet 5 · 2h")).toBeTruthy();
    expect(within(rowFor("Spike codex")).getByText("gpt-5-codex · 3d")).toBeTruthy();
  });

  it("filters on title or model, ignoring case, and Clear restores the list", () => {
    openPopover();
    fireEvent.change(filterBox(), { target: { value: "REFACTOR" } });
    expect(names()).toEqual(["Refactor auth"]);
    fireEvent.change(filterBox(), { target: { value: "gemini" } });
    expect(names()).toEqual(["Old terminal agent"]);
    fireEvent.change(filterBox(), { target: { value: "zzz" } });
    expect(screen.getByText('No matches for "zzz"')).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(names()).toHaveLength(3);
    expect(document.activeElement).toBe(filterBox());
  });

  it("Restore puts the agent in the focused pane, selected, and closes with composer focus pending", () => {
    openPopover();
    fireEvent.click(within(rowFor("Refactor auth")).getByRole("button", { name: "Restore Refactor auth" }));
    const ws = useWorkstreamTabsStore.getState().byWs.w1;
    expect(ws.shells.find((s) => s.id === 5)?.paneId).toBe("p2");
    expect(ws.panes.p2.activeId).toBe(5);
    expect(ws.activePaneId).toBe("p2");
    expect(archivedIds()).not.toContain(5);
    expect(useComposerFocusStore.getState().pendingShellId).toBe(5);
    expect(popover()).toBeNull();
  });

  it("confirms a delete, drops its transcript, and hands focus to the neighbouring row", async () => {
    openPopover();
    const confirm = askDelete("Spike codex");
    fireEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(archivedIds()).toEqual([5, 6, 7]);
    expect(h.dropSession).not.toHaveBeenCalled();
    await waitFor(() => expect(document.activeElement).toBe(rowFor("Spike codex")));

    fireEvent.click(within(askDelete("Spike codex")).getByRole("button", { name: "Delete" }));
    expect(archivedIds()).toEqual([5, 7]);
    expect(h.dropSession).toHaveBeenCalledWith("sess-6");
    await waitFor(() => expect(document.activeElement).toBe(rowFor("Refactor auth")));
  });

  it("drops no transcript for a PTY agent, and an emptied list focuses the filter", async () => {
    openPopover();
    fireEvent.click(within(askDelete("Old terminal agent")).getByRole("button", { name: "Delete" }));
    expect(archivedIds()).toEqual([5, 6]);
    expect(h.dropSession).not.toHaveBeenCalled();

    fireEvent.click(within(askDelete("Refactor auth")).getByRole("button", { name: "Delete" }));
    fireEvent.click(within(askDelete("Spike codex")).getByRole("button", { name: "Delete" }));
    expect(archivedIds()).toEqual([]);
    expect(screen.getByText("No archived sessions.")).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(filterBox()));
  });

  it("moves through rows with the keys, asks to delete on Backspace or Delete, and restores on Enter", async () => {
    openPopover();
    fireEvent.keyDown(filterBox(), { key: "ArrowDown" });
    expect(document.activeElement).toBe(rowFor("Old terminal agent"));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    expect(document.activeElement).toBe(filterBox());
    fireEvent.keyDown(filterBox(), { key: "ArrowDown" });
    fireEvent.keyDown(document.activeElement!, { key: "End" });
    expect(document.activeElement).toBe(rowFor("Spike codex"));
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    expect(document.activeElement).toBe(rowFor("Old terminal agent"));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement).toBe(rowFor("Refactor auth"));

    fireEvent.keyDown(rowFor("Refactor auth"), { key: "Backspace" });
    fireEvent.click(
      within(screen.getByRole("dialog", { name: "Delete Refactor auth?" })).getByRole("button", { name: "Cancel" }),
    );
    await waitFor(() => expect(document.activeElement).toBe(rowFor("Refactor auth")));
    fireEvent.keyDown(rowFor("Refactor auth"), { key: "Delete" });
    fireEvent.click(
      within(screen.getByRole("dialog", { name: "Delete Refactor auth?" })).getByRole("button", { name: "Cancel" }),
    );
    await waitFor(() => expect(document.activeElement).toBe(rowFor("Refactor auth")));
    expect(archivedIds()).toEqual([5, 6, 7]);

    fireEvent.keyDown(rowFor("Refactor auth"), { key: "Enter" });
    expect(archivedIds()).not.toContain(5);
    expect(popover()).toBeNull();
  });

  it("leaves keys aimed at a row's own buttons to those buttons", () => {
    openPopover();
    const row = rowFor("Spike codex");
    fireEvent.keyDown(within(row).getByRole("button", { name: "Delete Spike codex" }), { key: "Enter" });
    expect(archivedIds()).toContain(6);
    expect(popover()).not.toBeNull();
    fireEvent.keyDown(within(row).getByRole("button", { name: "Restore Spike codex" }), { key: "Backspace" });
    expect(screen.queryByRole("dialog", { name: /^Delete / })).toBeNull();
  });

  it("will not open while disconnected", () => {
    openPopover();
    fireEvent.click(trigger());
    expect(popover()).toBeNull();

    h.useConnectionStatus.mockReturnValue({ data: { state: "lost" } });
    fireEvent.click(trigger());
    expect(popover()).toBeNull();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
    expect(useAgentSessions.getState().open).toBe(false);
  });

  it("closes when the workstream changes", () => {
    const { rerender } = render(<AgentSessionsButton workstreamId="w1" />);
    fireEvent.click(trigger());
    expect(popover()).not.toBeNull();
    rerender(<AgentSessionsButton workstreamId="w2" />);
    expect(popover()).toBeNull();
  });

  it("toggles the archive on ⌘⇧A", () => {
    render(<AgentSessionsButton workstreamId="w1" />);
    act(() => {
      runCommand("view.toggleArchivedAgents");
    });
    expect(popover()).not.toBeNull();
    expect(screen.getByRole("radio", { name: "Archived 3" }).getAttribute("aria-checked")).toBe("true");
    act(() => {
      runCommand("view.toggleArchivedAgents");
    });
    expect(popover()).toBeNull();
  });
});
