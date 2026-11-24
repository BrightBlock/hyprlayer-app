// @vitest-environment jsdom
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ConnectionStatus, HostStats } from "@/lib/bindings";
import { ServerHostStats } from "./ServerHostStats";

const q = vi.hoisted(() => ({ useHostStats: vi.fn(), useActiveSessions: vi.fn(), useAllWorkstreams: vi.fn() }));
vi.mock("@/lib/queries", () => q);

const STATS: HostStats = {
  load: [9.6, 9, 8.5],
  cpus: 32,
  memoryUsed: 12e9,
  memoryTotal: 60e9,
  swapUsed: 0,
  swapTotal: 8e9,
  cpuPercent: 31,
  cpuHistory: [26, 29, 34, 31],
  memoryFree: 20e9,
};

const REMOTE: ConnectionStatus = {
  state: "connected",
  host: { destination: "jt@nexus-dev", label: "nexus-dev" },
  serverVersion: 3,
  serverRelease: null,
  serverOutdated: false,
  transcriptSync: false,
  error: null,
};

function mock(stats: HostStats | undefined) {
  q.useHostStats.mockReturnValue({ data: stats });
  q.useActiveSessions.mockReturnValue({ data: Array(4).fill({}) });
  q.useAllWorkstreams.mockReturnValue({ data: Array(9).fill({}) });
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerHostStats", () => {
  it("names a remote host, its destination and core count, then the counts and the host meters", () => {
    mock(STATS);
    render(<ServerHostStats connection={REMOTE} />);
    const section = screen.getByRole("region", { name: "Server stats for nexus-dev" });
    expect(within(section).getByText("remote")).toBeTruthy();
    expect(within(section).getByText("jt@nexus-dev")).toBeTruthy();
    expect(within(section).getByText("32 cores")).toBeTruthy();
    expect(within(section).getByText("Agents running").nextElementSibling?.textContent).toBe("4");
    expect(within(section).getByText("Workstreams").nextElementSibling?.textContent).toBe("9");
    expect(within(section).getByRole("meter", { name: "CPU usage" }).closest("[data-still-motion]")).not.toBeNull();
  });

  it("calls the local server this machine, with no remote badge", () => {
    mock(STATS);
    render(<ServerHostStats connection={{ ...REMOTE, host: null }} />);
    expect(screen.getByRole("region", { name: "Server stats for This machine" })).toBeTruthy();
    expect(screen.queryByText("remote")).toBeNull();
  });

  it("falls back to the text rows when the server reports no CPU percent", () => {
    mock({ ...STATS, cpuPercent: null, cpuHistory: null });
    render(<ServerHostStats connection={REMOTE} />);
    expect(screen.queryByRole("meter", { name: "CPU usage" })).toBeNull();
    expect(screen.getByText("Load").nextElementSibling?.textContent).toBe("9.60 9.00 8.50 (32 cores)");
  });
});
