// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LeadQueueItem, LeadView } from "@/lib/bindings";
import { LeadPanel } from "./LeadPanel";

const ipc = vi.hoisted(() => ({
  getBootstrap: vi.fn(),
  listLeads: vi.fn(),
  listLeadQueue: vi.fn(),
  removeLeadQueueItem: vi.fn(),
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
  ipc.hostStats.mockReturnValue(
    ok({ load: [1, 2, 3], cpus: 8, memoryUsed: 1e9, memoryTotal: 2e9, swapUsed: 0, swapTotal: 0 }),
  );
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
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

  it("removes the item whose remove button is pressed", async () => {
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Remove Billing export from the queue" }));
    await vi.waitFor(() => expect(ipc.removeLeadQueueItem).toHaveBeenCalledWith(PROJECT, "b"));
    expect(ipc.removeLeadQueueItem).toHaveBeenCalledTimes(1);
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
