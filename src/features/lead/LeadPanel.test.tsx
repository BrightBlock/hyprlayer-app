// @vitest-environment jsdom
import { QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LeadQueueItem, LeadView } from "@/lib/bindings";
import { queryClient } from "@/lib/queryClient";
import { LeadPanel } from "./LeadPanel";

const ipc = vi.hoisted(() => ({
  getBootstrap: vi.fn(),
  listLeads: vi.fn(),
  listLeadQueue: vi.fn(),
  removeLeadQueueItem: vi.fn(),
  reorderLeadQueue: vi.fn(),
  hostStats: vi.fn(),
}));
vi.mock("@/lib/bindings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/bindings")>();
  return { ...actual, commands: { ...actual.commands, ...ipc } };
});

const PROJECT = "/repo";
const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const item = (id: string, title: string, position: number): LeadQueueItem => ({
  id,
  projectPath: PROJECT,
  title,
  summary: `${title} summary`,
  state: "queued",
  workstreamId: null,
  position,
  createdAt: position,
});

const LEAD: LeadView = {
  projectPath: PROJECT,
  workstreamId: "main",
  sessionId: "lead-session",
  driving: [],
  liveChildren: 3,
  projectRules: null,
  remoteControl: false,
  remoteControlUrl: null,
  summary: null,
};

beforeEach(() => {
  ipc.getBootstrap.mockReturnValue(ok({ settings: { agentControl: { lead: { enabled: true, maxLiveChildren: 6 } } } }));
  ipc.listLeads.mockReturnValue(ok([LEAD]));
  ipc.listLeadQueue.mockReturnValue(ok([item("a", "Login page", 0), item("b", "Billing export", 1)]));
  ipc.removeLeadQueueItem.mockReturnValue(ok(null));
  ipc.reorderLeadQueue.mockReturnValue(ok(null));
  ipc.hostStats.mockReturnValue(
    ok({ load: [1, 2, 3], cpus: 8, memoryUsed: 1e9, memoryTotal: 2e9, swapUsed: 0, swapTotal: 0 }),
  );
});

afterEach(() => {
  cleanup();
  queryClient.clear();
  vi.clearAllMocks();
});

function renderPanel() {
  // The app's own client: the queue's mutations update it directly.
  render(
    <QueryClientProvider client={queryClient}>
      <LeadPanel projectPath={PROJECT} views={[]} />
    </QueryClientProvider>,
  );
}

describe("LeadPanel", () => {
  it("lists queued items in order", async () => {
    renderPanel();
    const first = await screen.findByText("Login page");
    const second = screen.getByText("Billing export");
    expect(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText("Login page summary")).toBeTruthy();
  });

  it("keeps the tour's queue anchor when nothing is queued", async () => {
    ipc.listLeadQueue.mockReturnValue(ok([]));
    renderPanel();
    await screen.findByText("Nothing queued");
    expect(document.querySelector('[data-tour="lead.panel.queue"]')?.textContent).toContain("Nothing queued");
  });

  it("removes the item whose remove button is pressed", async () => {
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Remove Billing export from the queue" }));
    await vi.waitFor(() => expect(ipc.removeLeadQueueItem).toHaveBeenCalledWith(PROJECT, "b"));
    expect(ipc.removeLeadQueueItem).toHaveBeenCalledTimes(1);
  });

  it("moves an item down with the arrow key on its handle, and keeps focus on it", async () => {
    ipc.reorderLeadQueue.mockImplementation(() => {
      ipc.listLeadQueue.mockReturnValue(ok([item("b", "Billing export", 0), item("a", "Login page", 1)]));
      return ok(null);
    });
    renderPanel();
    const handle = await screen.findByRole("button", { name: "Reorder Login page, position 1 of 2" });
    handle.focus();
    fireEvent.keyDown(handle, { key: "ArrowDown" });
    await vi.waitFor(() => expect(ipc.reorderLeadQueue).toHaveBeenCalledWith(PROJECT, ["b", "a"]));
    const moved = await screen.findByRole("button", { name: "Reorder Login page, position 2 of 2" });
    expect(document.activeElement).toBe(moved);
    expect(screen.getByRole("status").textContent).toBe("Login page moved to position 2 of 2");
  });

  it("ignores a move past either end", async () => {
    renderPanel();
    const handle = await screen.findByRole("button", { name: "Reorder Login page, position 1 of 2" });
    fireEvent.keyDown(handle, { key: "ArrowUp" });
    fireEvent.keyDown(handle, { key: "Home" });
    expect(ipc.reorderLeadQueue).not.toHaveBeenCalled();
  });

  it("puts the server's order back when the reorder is refused", async () => {
    ipc.reorderLeadQueue.mockReturnValue(
      Promise.resolve({ status: "error" as const, error: { code: "internal", message: "no", remedy: null } }),
    );
    renderPanel();
    fireEvent.keyDown(await screen.findByRole("button", { name: "Reorder Login page, position 1 of 2" }), {
      key: "End",
    });
    await vi.waitFor(() => expect(ipc.reorderLeadQueue).toHaveBeenCalledWith(PROJECT, ["b", "a"]));
    expect(await screen.findByRole("button", { name: "Reorder Login page, position 1 of 2" })).toBeTruthy();
  });

  it("shows live agents against the Lead's limit", async () => {
    renderPanel();
    expect(await screen.findByText("Agents")).toBeTruthy();
    expect(screen.getByText("3/6")).toBeTruthy();
  });

  it("falls back to text rows when the server sends no CPU usage", async () => {
    renderPanel();
    expect(await screen.findByText("Host")).toBeTruthy();
    expect(screen.getByText("1.00 2.00 3.00 (8 cores)")).toBeTruthy();
    expect(screen.getByText("1.0 GB of 2.0 GB")).toBeTruthy();
    expect(screen.queryByRole("meter")).toBeNull();
  });

  it("draws meters when the server sends CPU usage", async () => {
    ipc.hostStats.mockReturnValue(
      ok({
        load: [1, 2, 3],
        cpus: 8,
        memoryUsed: 1e9,
        memoryTotal: 2e9,
        swapUsed: 0,
        swapTotal: 0,
        cpuPercent: 40,
        cpuHistory: [40],
        memoryFree: 5e8,
      }),
    );
    renderPanel();
    expect(await screen.findByRole("meter", { name: "CPU usage" })).toBeTruthy();
    expect(screen.getByText("8 cores")).toBeTruthy();
    expect(screen.getByText("none on this host")).toBeTruthy();
  });
});
