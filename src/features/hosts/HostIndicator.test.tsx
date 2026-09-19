// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ConnectionStatus, Host } from "@/lib/bindings";
import { HostIndicator } from "./HostIndicator";
import { useHostsUiStore } from "./hostsUi";

const b = vi.hoisted(() => ({ useConnectionStatus: vi.fn() }));
vi.mock("@/lib/queries", () => ({ useConnectionStatus: b.useConnectionStatus }));

const NEXUS: Host = { destination: "nexus", label: "Nexus" };

function mockStatus(data: ConnectionStatus | undefined) {
  b.useConnectionStatus.mockReturnValue({ data, isSuccess: data !== undefined });
}

beforeEach(() => {
  useHostsUiStore.setState({ open: false });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("HostIndicator", () => {
  it("opens the deliberate hosts dialog mode on click", () => {
    mockStatus({ state: "connected", host: null, serverVersion: 3, serverRelease: null, serverOutdated: false, error: null });
    render(<HostIndicator />);
    fireEvent.click(screen.getByRole("button", { name: "Server: this machine" }));
    expect(useHostsUiStore.getState().open).toBe(true);
  });

  it("reads 'Server: this machine' and shows no dot when local and connected", () => {
    mockStatus({ state: "connected", host: null, serverVersion: 3, serverRelease: null, serverOutdated: false, error: null });
    render(<HostIndicator />);
    const button = screen.getByRole("button", { name: "Server: this machine" });
    expect(button.title).toBe("Server: this machine");
    expect(button.querySelector("span[aria-hidden]")).toBeNull();
  });

  it("names the host and shows a dot when remote and connected", () => {
    mockStatus({ state: "connected", host: NEXUS, serverVersion: 3, serverRelease: null, serverOutdated: false, error: null });
    render(<HostIndicator />);
    const button = screen.getByRole("button", { name: "Server: Nexus" });
    expect(button.querySelector("span[aria-hidden]")).not.toBeNull();
  });

  it("names the host and shows a dot when failed", () => {
    mockStatus({
      state: "failed",
      host: NEXUS,
      serverVersion: null,
      serverRelease: null, serverOutdated: false,
      error: { code: "remote_unreachable", message: "Could not reach nexus.", remedy: null },
    });
    render(<HostIndicator />);
    const button = screen.getByRole("button", { name: "Not connected to Nexus" });
    expect(button.querySelector("span[aria-hidden]")).not.toBeNull();
  });

  it("names the host and shows a dot when lost", () => {
    mockStatus({
      state: "lost",
      host: NEXUS,
      serverVersion: 3,
      serverRelease: null, serverOutdated: false,
      error: { code: "server_disconnected", message: "The server connection ended.", remedy: null },
    });
    render(<HostIndicator />);
    const button = screen.getByRole("button", { name: "Connection to Nexus lost" });
    expect(button.querySelector("span[aria-hidden]")).not.toBeNull();
  });

  it("renders the quiet local-connected appearance before the status resolves", () => {
    mockStatus(undefined);
    render(<HostIndicator />);
    const button = screen.getByRole("button", { name: "Server: this machine" });
    expect(button.querySelector("span[aria-hidden]")).toBeNull();
  });
});
