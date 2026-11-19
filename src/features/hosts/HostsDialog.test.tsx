// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError, ConnectionStatus, Host, HostProvisioning, HostsFile } from "@/lib/bindings";
import { ESCAPE_COMMAND_ID, runCommand } from "@/lib/keybindings/registry";
import { HostsDialog } from "./HostsDialog";
import { useHostsUiStore } from "./hostsUi";
import { useServerAutoUpdate } from "./serverAutoUpdate";

const b = vi.hoisted(() => ({
  useConnectionStatus: vi.fn(),
  useHosts: vi.fn(),
  useSaveHosts: vi.fn(),
  useTestHostConnection: vi.fn(),
  useHostProvisioning: vi.fn(),
  useLocalServers: vi.fn(),
  relaunch: vi.fn(() => Promise.resolve()),
  flushPersistedWrites: vi.fn(),
}));
vi.mock("@/lib/queries", () => ({
  useConnectionStatus: b.useConnectionStatus,
  useHosts: b.useHosts,
  useSaveHosts: b.useSaveHosts,
  useTestHostConnection: b.useTestHostConnection,
  useHostProvisioning: b.useHostProvisioning,
  useLocalServers: b.useLocalServers,
  useHostProbes: () => ({}),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: b.relaunch }));
vi.mock("@/lib/persistedStorage", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/persistedStorage")>()),
  flushPersistedWrites: b.flushPersistedWrites,
}));

const h = vi.hoisted(() => {
  class FakeChannel {
    onmessage: ((step: unknown) => void) | null = null;
  }
  const provisionHost = vi.fn(async () => ({ status: "ok", data: null }));
  const updateHostServer = vi.fn(async () => ({ status: "ok", data: null }));
  return { FakeChannel, provisionHost, updateHostServer };
});
vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));

const reconnect = vi.hoisted(() => ({ retryConnection: vi.fn() }));
vi.mock("./reconnect", () => reconnect);
vi.mock("@/lib/bindings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/bindings")>();
  return {
    ...actual,
    commands: {
      ...actual.commands,
      provisionHost: h.provisionHost,
      updateHostServer: h.updateHostServer,
    },
  };
});

const NEXUS: Host = { destination: "nexus", label: "Nexus" };

const UNREACHABLE: AppError = {
  code: "remote_unreachable",
  message: "Could not reach nexus over SSH.",
  remedy: "Check that the machine is awake and reachable.",
};

const NO_SERVER: AppError = {
  code: "remote_cli_missing",
  message: "Nexus does not have the Hyprlayer server this build needs.",
  remedy: "Install it from the Hyprlayer server panel.",
};

const PROVISIONING: HostProvisioning = {
  version: "0.7.5",
  triple: "x86_64-unknown-linux-gnu",
  state: "missing",
  installedVersions: [],
  usableVersion: null,
  source: "cache",
};

const LOCAL_DOWN: AppError = {
  code: "server_unavailable",
  message: "The local Hyprlayer server did not start.",
  remedy: null,
};

const save = vi.fn();
const saveAsync = vi.fn(() => Promise.resolve());

function mockStatus(data: ConnectionStatus | undefined) {
  b.useConnectionStatus.mockReturnValue({
    data,
    isSuccess: data !== undefined,
    isLoading: data === undefined,
    isError: false,
    error: null,
  });
}

function mockHosts(file: HostsFile) {
  b.useHosts.mockReturnValue({ data: file, isSuccess: true, isLoading: false, error: null });
}

beforeEach(() => {
  b.useSaveHosts.mockReturnValue({
    mutate: save,
    mutateAsync: saveAsync,
    isPending: false,
    error: null,
  });
  b.useTestHostConnection.mockReturnValue({ mutateAsync: vi.fn() });
  b.useHostProvisioning.mockReturnValue({ data: undefined });
  b.useLocalServers.mockReturnValue({ data: [] });
  mockHosts({ hosts: [], active: null });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useHostsUiStore.setState({ open: false });
  useServerAutoUpdate.setState({ updating: false });
});

describe("HostsDialog", () => {
  it("renders nothing before the first connection_status resolves", () => {
    mockStatus(undefined);
    const { container } = render(<HostsDialog />);
    expect(container.innerHTML).toBe("");
  });

  it("renders nothing on a healthy connection", () => {
    mockStatus({ state: "connected", host: null, serverVersion: 3, serverRelease: null, serverOutdated: false, transcriptSync: false, error: null });
    const { container } = render(<HostsDialog />);
    expect(container.innerHTML).toBe("");
  });

  it("opens with no host configured and a failed local server", () => {
    mockStatus({ state: "failed", host: null, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: LOCAL_DOWN });
    render(<HostsDialog />);

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText("Not connected")).toBeTruthy();
    expect(screen.getByText("This machine")).toBeTruthy();
    expect(screen.getByText(LOCAL_DOWN.message)).toBeTruthy();
    expect(screen.getByText(/No hosts yet/)).toBeTruthy();

    const useLocal = screen.getByRole("button", { name: "Use this machine" }) as HTMLButtonElement;
    expect(useLocal.disabled).toBe(true);
  });

  it("opens with a configured host that cannot be reached, naming the reason", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: UNREACHABLE });
    mockHosts({ hosts: [NEXUS], active: "nexus" });
    render(<HostsDialog />);

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText("Not connected").parentElement?.textContent).toContain("Nexus");
    expect(screen.getByText(UNREACHABLE.message)).toBeTruthy();
    expect(screen.getByText(UNREACHABLE.remedy!)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Use this machine" }) as HTMLButtonElement).disabled).toBe(
      false,
    );
  });

  it("distinguishes a connection that was lost from one never made, once opened", () => {
    mockStatus({
      state: "lost",
      host: NEXUS,
      serverVersion: 3,
      serverRelease: null, serverOutdated: false, transcriptSync: false,
      error: { code: "server_disconnected", message: "The server connection ended.", remedy: null },
    });
    mockHosts({ hosts: [NEXUS], active: "nexus" });
    useHostsUiStore.setState({ open: true });
    render(<HostsDialog />);

    expect(screen.getByText("Connection lost")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Relaunch" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close dialog" })).toBeTruthy();
  });

  it("does not close on Escape in blocking mode, and still consumes the key", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: UNREACHABLE });
    mockHosts({ hosts: [NEXUS], active: "nexus" });
    render(<HostsDialog />);

    expect(runCommand(ESCAPE_COMMAND_ID)).toBe(true);

    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("confirms before switching, then writes, flushes and relaunches", async () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: UNREACHABLE });
    mockHosts({ hosts: [NEXUS], active: "nexus" });
    render(<HostsDialog />);

    fireEvent.click(screen.getByRole("button", { name: "Use this machine" }));

    // A switch restarts the app, so it is never the first click.
    expect(saveAsync).not.toHaveBeenCalled();
    expect(screen.getByText("Switch server?")).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Switch and restart" }));
    });

    expect(saveAsync).toHaveBeenCalledWith({ hosts: [NEXUS], active: null, localProfile: null });
    expect(b.flushPersistedWrites).toHaveBeenCalledTimes(1);
    expect(b.relaunch).toHaveBeenCalledTimes(1);
  });

  it("says the old server's agents keep running, but only when one is live", async () => {
    mockStatus({ state: "connected", host: null, serverVersion: 3, serverRelease: null, serverOutdated: false, transcriptSync: false, error: null });
    mockHosts({ hosts: [NEXUS], active: null });
    useHostsUiStore.setState({ open: true });
    render(<HostsDialog />);

    fireEvent.click(screen.getByTitle("Use Nexus"));

    expect(screen.getByText(/restart on/).textContent).toContain("Nexus");
    expect(screen.getByText(/keep running/)).toBeTruthy();

    // Cancelling writes nothing and restarts nothing.
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(saveAsync).not.toHaveBeenCalled();
    expect(b.relaunch).not.toHaveBeenCalled();
  });

  it("offers the install when the host has no server, naming the version once", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: NO_SERVER });
    mockHosts({ hosts: [NEXUS], active: "nexus" });
    b.useHostProvisioning.mockReturnValue({ data: PROVISIONING });
    render(<HostsDialog />);

    // The header already carries the badge, the host and the error; a fresh
    // install adds nothing but the action, and names the host nowhere again.
    expect(screen.getByRole("button", { name: "Install server 0.7.5" })).toBeTruthy();
    expect(screen.queryByText(/installs into/)).toBeNull();
    expect(screen.queryByText(/copied across/)).toBeNull();
    // The remedy points at this very panel, so the header suppresses it.
    expect(screen.queryByText(/Install it from the Hyprlayer server panel/)).toBeNull();
    expect(h.provisionHost).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Install server 0.7.5" }));

    expect(h.provisionHost).toHaveBeenCalledWith(NEXUS, expect.anything());
    // On the host's own row, not in a panel beside the list.
    expect(screen.getByText(/Checking the host/).closest("li")?.textContent).toContain("Nexus");
  });

  it("installs without asking on a host that already carries one of our servers", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: NO_SERVER });
    b.useHostProvisioning.mockReturnValue({
      data: { ...PROVISIONING, installedVersions: ["0.7.4"] },
    });
    render(<HostsDialog />);

    expect(h.provisionHost).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /Install server/ })).toBeNull();
    // A version change says nothing here; the switch confirm is where the user
    // chooses to end those sessions.
    expect(screen.queryByText(/Agents and terminals/)).toBeNull();
  });

  it("reconnects once the install lands, rather than leaving the stale failure up", async () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: NO_SERVER });
    mockHosts({ hosts: [NEXUS], active: "nexus" });
    b.useHostProvisioning.mockReturnValue({ data: PROVISIONING });
    render(<HostsDialog />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Install server 0.7.5" }));
    });

    // The header's message is the one this install just fixed, so the app
    // re-dials rather than asking for a relaunch to find that out.
    expect(reconnect.retryConnection).toHaveBeenCalledTimes(1);
    expect(b.relaunch).not.toHaveBeenCalled();
  });

  it("reinstalls a same-version binary whose bytes are not ours, without asking", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: NO_SERVER });
    b.useHostProvisioning.mockReturnValue({
      data: { ...PROVISIONING, state: "stale", installedVersions: ["0.7.5"] },
    });
    render(<HostsDialog />);

    // Same version, different bytes — non-obvious, so it is said.
    expect(
      screen.getByText("A different build of 0.7.5 is installed; it will be replaced."),
    ).toBeTruthy();
    expect(h.provisionHost).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /Install server/ })).toBeNull();
  });

  it("offers nothing when the host is ready, or failed for another reason", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: NO_SERVER });
    b.useHostProvisioning.mockReturnValue({ data: { ...PROVISIONING, state: "ready" } });
    const ready = render(<HostsDialog />);
    expect(ready.container.textContent).not.toContain("Install server");
    cleanup();

    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: UNREACHABLE });
    b.useHostProvisioning.mockReturnValue({ data: PROVISIONING });
    render(<HostsDialog />);
    expect(screen.queryByRole("button", { name: /Install server/ })).toBeNull();
  });

  it("flushes the debounced writes before relaunching", () => {
    mockStatus({ state: "failed", host: NEXUS, serverVersion: null, serverRelease: null, serverOutdated: false, transcriptSync: false, error: UNREACHABLE });
    render(<HostsDialog />);

    fireEvent.click(screen.getByRole("button", { name: "Relaunch" }));

    expect(b.flushPersistedWrites).toHaveBeenCalledTimes(1);
    expect(b.relaunch).toHaveBeenCalledTimes(1);
    expect(b.flushPersistedWrites.mock.invocationCallOrder[0]).toBeLessThan(
      b.relaunch.mock.invocationCallOrder[0],
    );
  });

  it("offers no relaunch while connected", () => {
    mockStatus({ state: "connected", host: null, serverVersion: 3, serverRelease: null, serverOutdated: false, transcriptSync: false, error: null });
    useHostsUiStore.setState({ open: true });
    render(<HostsDialog />);
    expect(screen.queryByRole("button", { name: "Relaunch" })).toBeNull();
  });

  it("shows the restart on the header line until the server comes back", async () => {
    const connected: ConnectionStatus = {
      state: "connected",
      host: null,
      serverVersion: 3,
      serverRelease: null, serverOutdated: true, transcriptSync: false,
      error: null,
    };
    mockStatus(connected);
    useHostsUiStore.setState({ open: true });
    const restartServer = vi.fn(async () => ({ status: "ok", data: null }));
    const bindings = await import("@/lib/bindings");
    vi.spyOn(bindings.commands, "restartServer").mockImplementation(restartServer as never);
    const { rerender } = render(<HostsDialog />);

    fireEvent.click(screen.getByRole("button", { name: "Restart server" }));
    const [, confirm] = screen.getAllByRole("button", { name: "Restart server" });
    await act(async () => {
      fireEvent.click(confirm);
    });
    expect(restartServer).toHaveBeenCalledTimes(1);
    expect(screen.getByText("This machine").parentElement?.textContent).toContain("Restarting…");

    mockStatus({ ...connected, state: "lost", serverOutdated: false });
    rerender(<HostsDialog />);
    expect(screen.getByText(/Restarting…/)).toBeTruthy();

    mockStatus({ ...connected, serverOutdated: false });
    rerender(<HostsDialog />);
    expect(screen.queryByText(/Restarting…/)).toBeNull();
  });
  it("updates a remote server below the minimum, then reconnects", async () => {
    mockStatus({
      state: "failed",
      host: NEXUS,
      serverVersion: null,
      serverRelease: null,
      serverOutdated: false,
      transcriptSync: false,
      error: {
        code: "remote_server_outdated",
        message: "Nexus is running Hyprlayer server 0.8.2; this app needs 0.9.0 or newer.",
        remedy: "Update it from the Hyprlayer server panel.",
      },
    });
    render(<HostsDialog />);
    expect(screen.queryByText("Update it from the Hyprlayer server panel.")).toBeNull();
    expect(screen.queryByText(/this app ships a newer one/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Update server" }));
    const [, confirm] = screen.getAllByRole("button", { name: "Update server" });
    await act(async () => {
      fireEvent.click(confirm);
    });
    expect(h.updateHostServer).toHaveBeenCalledTimes(1);
    expect(reconnect.retryConnection).toHaveBeenCalledTimes(1);
  });

  it("offers an optional update on a connected remote host, naming its version", async () => {
    mockStatus({
      state: "connected",
      host: NEXUS,
      serverVersion: 1,
      serverRelease: "0.8.3",
      serverOutdated: true,
      transcriptSync: false,
      error: null,
    });
    useHostsUiStore.setState({ open: true });
    render(<HostsDialog />);
    expect(screen.getByText(/running 0\.8\.3/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Restart server" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Update server" }));
    const [, confirm] = screen.getAllByRole("button", { name: "Update server" });
    await act(async () => {
      fireEvent.click(confirm);
    });
    expect(h.updateHostServer).toHaveBeenCalledTimes(1);
    // The dropped connection's own reconnect loop takes it from here.
    expect(reconnect.retryConnection).not.toHaveBeenCalled();
  });

  it.each([true, false])("never opens on its own for a dropped connection (updating: %s)", (updating) => {
    useServerAutoUpdate.setState({ updating });
    mockStatus({
      state: "lost",
      host: null,
      serverVersion: null,
      serverRelease: null,
      serverOutdated: false,
      transcriptSync: false,
      error: { code: "server_disconnected", message: "The server connection ended.", remedy: null },
    });
    const { container } = render(<HostsDialog />);
    expect(container.innerHTML).toBe("");
  });

  it("keeps a refused host's dialog up while it updates", () => {
    useServerAutoUpdate.setState({ updating: true });
    mockStatus({
      state: "failed",
      host: NEXUS,
      serverVersion: null,
      serverRelease: null,
      serverOutdated: false,
      transcriptSync: false,
      error: {
        code: "remote_server_outdated",
        message: "Nexus is running Hyprlayer server 0.8.10; this app needs 0.8.11 or newer.",
        remedy: null,
      },
    });
    render(<HostsDialog />);
    expect(screen.getByText("Not connected")).toBeTruthy();
    expect(screen.getByText("Updating…")).toBeTruthy();
  });
});
