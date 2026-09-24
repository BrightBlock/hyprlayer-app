// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ConnectionStatus, Host, HostProbe, HostsFile, WorkstreamView } from "@/lib/bindings";
import type { HostProbeResult } from "@/lib/domain/hosts";
import { useHostsUiStore } from "./hostsUi";
import { ServersSettings } from "./ServersSettings";

const q = vi.hoisted(() => ({
  useHosts: vi.fn(),
  useConnectionStatus: vi.fn(),
  useSaveHosts: vi.fn(),
  useHostProbes: vi.fn(),
  useTestHostConnection: vi.fn(),
  useAllWorkstreams: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);

// A real relaunch would end the test run; only the order of the two calls matters.
const app = vi.hoisted(() => ({ relaunch: vi.fn(() => Promise.resolve()), flushPersistedWrites: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: app.relaunch }));
vi.mock("@/lib/persistedStorage", () => ({ flushPersistedWrites: app.flushPersistedWrites }));

const install = vi.hoisted(() => ({ runProvision: vi.fn() }));
vi.mock("./serverInstall", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./serverInstall")>()),
  runProvision: install.runProvision,
}));

const reconnect = vi.hoisted(() => ({ retryConnection: vi.fn() }));
vi.mock("./reconnect", () => reconnect);

const NEXUS: Host = { destination: "nexus", label: "Nexus" };
const LAPTOP: Host = { destination: "jt@laptop.local", label: "Laptop" };

const saveAsync = vi.fn();
const testAsync = vi.fn();

function status(
  state: ConnectionStatus["state"],
  host: Host | null,
  extra: Partial<ConnectionStatus> = {},
): ConnectionStatus {
  return {
    state,
    host,
    serverVersion: state === "connected" ? 1 : null,
    serverRelease: null,
    serverOutdated: false,
    error: null,
    ...extra,
  };
}

function setup({
  file = { hosts: [NEXUS, LAPTOP], active: "nexus", localProfile: null },
  connection = status("connected", NEXUS),
  probes = {},
  workstreams = [],
}: {
  file?: HostsFile;
  connection?: ConnectionStatus;
  probes?: Record<string, HostProbeResult>;
  workstreams?: WorkstreamView[];
} = {}) {
  q.useHosts.mockReturnValue({ data: file, isSuccess: true });
  q.useConnectionStatus.mockReturnValue({ data: connection, isSuccess: true });
  q.useHostProbes.mockReturnValue(probes);
  q.useAllWorkstreams.mockReturnValue({ data: workstreams });
  return render(<ServersSettings />);
}

const row = (name: string) => {
  const li = screen.getByText(name, { exact: true }).closest("li");
  if (!li) throw new Error(`no row for ${name}`);
  return li;
};
const destination = () => screen.getByLabelText("SSH destination") as HTMLInputElement;
const addServer = () => screen.getByRole("button", { name: "Add server" }) as HTMLButtonElement;

function type(input: HTMLInputElement, text: string) {
  fireEvent.change(input, { target: { value: text } });
}

function worktree(mode: WorkstreamView["item"]["worktreeMode"]): WorkstreamView {
  return { item: { worktreeMode: mode } } as WorkstreamView;
}

beforeEach(() => {
  saveAsync.mockResolvedValue(null);
  testAsync.mockResolvedValue({ ok: true, serverVersion: 1, error: null } satisfies HostProbe);
  q.useSaveHosts.mockReturnValue({
    mutate: vi.fn(),
    mutateAsync: saveAsync,
    isPending: false,
    error: null,
    reset: vi.fn(),
  });
  q.useTestHostConnection.mockReturnValue({ mutateAsync: testAsync });
  install.runProvision.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useHostsUiStore.setState({ open: false });
});

describe("ServersSettings", () => {
  it("asks before removing a server, and writes nothing until confirmed", async () => {
    setup();

    fireEvent.click(within(row("Laptop")).getByRole("button", { name: "Remove" }));

    expect(saveAsync).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Remove server?")).toBeTruthy();
    expect(dialog.textContent).toContain("Hyprlayer forgets Laptop.");

    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    });

    expect(saveAsync).toHaveBeenCalledWith({ hosts: [NEXUS], active: "nexus", localProfile: null });
  });

  it("writes nothing when a removal is cancelled", () => {
    setup();

    fireEvent.click(within(row("Laptop")).getByRole("button", { name: "Remove" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(saveAsync).not.toHaveBeenCalled();
  });

  it("does not offer to remove the default server", () => {
    setup();
    expect(within(row("Nexus")).queryByRole("button", { name: "Remove" })).toBeNull();
    expect(within(row("Nexus")).queryByRole("button", { name: "Set default" })).toBeNull();
  });

  it("confirms before setting a default, then writes, flushes and relaunches", async () => {
    setup();

    fireEvent.click(within(row("Laptop")).getByRole("button", { name: "Set default" }));

    // A switch restarts the app, so it is never the first click.
    expect(saveAsync).not.toHaveBeenCalled();
    expect(app.relaunch).not.toHaveBeenCalled();
    expect(screen.getByText(/restart on/).textContent).toContain("Laptop");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Switch and restart" }));
    });

    expect(saveAsync).toHaveBeenCalledWith({
      hosts: [NEXUS, LAPTOP],
      active: "jt@laptop.local",
      localProfile: null,
    });
    expect(app.flushPersistedWrites).toHaveBeenCalledTimes(1);
    expect(app.relaunch).toHaveBeenCalledTimes(1);
    expect(app.flushPersistedWrites.mock.invocationCallOrder[0]).toBeLessThan(
      app.relaunch.mock.invocationCallOrder[0],
    );
  });

  it("does not relaunch when the switch cannot be written", async () => {
    saveAsync.mockRejectedValue(new Error("disk full"));
    setup();

    fireEvent.click(within(row("Laptop")).getByRole("button", { name: "Set default" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Switch and restart" }));
    });

    expect(app.relaunch).not.toHaveBeenCalled();
  });

  it("switches back to this machine by clearing the active host", async () => {
    setup({ file: { hosts: [NEXUS], active: "nexus", localProfile: "/other/profile" } });

    fireEvent.click(within(row("This machine")).getByRole("button", { name: "Set default" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Switch and restart" }));
    });

    expect(saveAsync).toHaveBeenCalledWith({ hosts: [NEXUS], active: null, localProfile: null });
    expect(app.relaunch).toHaveBeenCalledTimes(1);
  });

  it("offers this machine neither Edit nor Remove, whether or not it is the default", () => {
    setup();
    expect(within(row("This machine")).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(row("This machine")).queryByRole("button", { name: "Remove" })).toBeNull();
    cleanup();

    setup({ file: { hosts: [NEXUS], active: null }, connection: status("connected", null) });
    expect(row("This machine").textContent).toContain("DEFAULT");
    expect(within(row("This machine")).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(row("This machine")).queryByRole("button", { name: "Remove" })).toBeNull();
  });

  it("marks this machine as the default when the active host is not listed", () => {
    setup({ file: { hosts: [NEXUS], active: "gone" }, connection: status("connected", null) });
    expect(row("This machine").textContent).toContain("DEFAULT");
    expect(row("Nexus").textContent).not.toContain("DEFAULT");
  });

  it("refuses a destination ssh would read as an option", () => {
    setup();

    type(destination(), "-oProxyCommand=touch${IFS}/tmp/x");

    expect(screen.getByRole("alert").textContent).toContain('"-"');
    expect(addServer().disabled).toBe(true);
    fireEvent.click(addServer());
    expect(saveAsync).not.toHaveBeenCalled();
  });

  it("refuses a URL, pointing at the SSH form instead", () => {
    setup();

    for (const url of ["https://nexus.local", "ws://nexus:9000"]) {
      type(destination(), url);
      const alert = screen.getByRole("alert").textContent ?? "";
      expect(alert).toContain("not a URL");
      expect(alert).toContain("user@host");
      expect(addServer().disabled).toBe(true);
    }
    expect(saveAsync).not.toHaveBeenCalled();
  });

  it("keeps Add disabled, without complaining, while the destination is blank", () => {
    setup();
    type(screen.getByLabelText("Label") as HTMLInputElement, "Box");
    expect(addServer().disabled).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("refuses a destination another server already uses", () => {
    setup();
    type(destination(), " nexus ");
    expect(screen.getByRole("alert").textContent).toMatch(/already uses/);
    expect(addServer().disabled).toBe(true);
  });

  it("adds an unlabelled server under its destination, then tests it once", async () => {
    setup();

    type(destination(), " ssh://jt@box:2222 ");
    await act(async () => {
      fireEvent.click(addServer());
    });

    const box = { destination: "ssh://jt@box:2222", label: "ssh://jt@box:2222" };
    expect(saveAsync).toHaveBeenCalledWith({
      hosts: [NEXUS, LAPTOP, box],
      active: "nexus",
      localProfile: null,
    });
    await waitFor(() => expect(testAsync).toHaveBeenCalledTimes(1));
    expect(testAsync).toHaveBeenCalledWith(box);
    expect(destination().value).toBe("");
  });

  it("does not test a server whose add could not be written", async () => {
    saveAsync.mockRejectedValue(new Error("disk full"));
    setup();

    type(destination(), "jt@box");
    await act(async () => {
      fireEvent.click(addServer());
    });

    expect(testAsync).not.toHaveBeenCalled();
    expect(destination().value).toBe("jt@box");
  });

  it("renders an unreachable probe as a warn row carrying its short reason", () => {
    setup({
      connection: status("connected", null),
      file: { hosts: [NEXUS], active: null },
      probes: { nexus: { ok: false, code: "remote_auth_failed", at: 1 } },
    });

    const nexus = row("Nexus");
    expect(nexus.className).toContain("bg-warn/5");
    const reason = within(nexus).getByText("nexus · Auth failed");
    expect(reason.className).toContain("text-warn-ink");
    expect(within(nexus).getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(within(nexus).queryByRole("button", { name: "Set default" })).toBeNull();
  });

  it("leaves an untested server unmarked rather than unreachable", () => {
    setup({ connection: status("connected", null), file: { hosts: [NEXUS], active: null } });

    const nexus = row("Nexus");
    expect(nexus.className).not.toContain("bg-warn/5");
    expect(within(nexus).getByText("nexus")).toBeTruthy();
    expect(within(nexus).queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("marks the connected server unreachable once its connection is lost, whatever a probe said", () => {
    setup({
      connection: status("lost", NEXUS, {
        error: { code: "server_disconnected", message: "The server connection ended.", remedy: null },
      }),
      probes: { nexus: { ok: true, at: 1 } },
    });

    expect(row("Nexus").className).toContain("bg-warn/5");
    expect(within(row("Nexus")).getByText("nexus · Disconnected")).toBeTruthy();
  });

  it("retries an unreachable server with a fresh probe", async () => {
    setup({
      connection: status("connected", null),
      file: { hosts: [NEXUS], active: null },
      probes: { nexus: { ok: false, code: "remote_unreachable", at: 1 } },
    });

    await act(async () => {
      fireEvent.click(within(row("Nexus")).getByRole("button", { name: "Retry" }));
    });

    expect(testAsync).toHaveBeenCalledWith(NEXUS);
  });

  it("offers Install only for a missing server, through the provisioning flow", async () => {
    setup({
      connection: status("connected", null),
      file: { hosts: [NEXUS, LAPTOP], active: null },
      probes: {
        nexus: { ok: false, code: "remote_cli_missing", at: 1 },
        "jt@laptop.local": { ok: false, code: "remote_auth_failed", at: 1 },
      },
    });

    expect(within(row("Laptop")).queryByRole("button", { name: "Install" })).toBeNull();
    await act(async () => {
      fireEvent.click(within(row("Nexus")).getByRole("button", { name: "Install" }));
    });

    expect(install.runProvision).toHaveBeenCalledWith(NEXUS, expect.any(Function));
    // Installing is not choosing: nothing is written and nothing restarts.
    expect(saveAsync).not.toHaveBeenCalled();
    expect(app.relaunch).not.toHaveBeenCalled();
    // The probe after the push is what says it worked.
    await waitFor(() => expect(testAsync).toHaveBeenCalledWith(NEXUS));
  });

  it("sends an outdated connected server's update to the hosts dialog", () => {
    setup({ connection: status("connected", NEXUS, { serverOutdated: true, serverRelease: "0.8.3" }) });

    expect(within(row("Laptop")).queryByRole("button", { name: "Update" })).toBeNull();
    fireEvent.click(within(row("Nexus")).getByRole("button", { name: "Update" }));

    expect(useHostsUiStore.getState().open).toBe(true);
  });

  it("counts worktrees on the connected server only", () => {
    setup({
      workstreams: [worktree("isolated"), worktree("existing"), worktree("inplace"), worktree("folder")],
    });

    expect(within(row("Nexus")).getByText("nexus · 2 worktrees")).toBeTruthy();
    expect(within(row("Laptop")).queryByText(/worktree/)).toBeNull();
    expect(within(row("This machine")).queryByText(/worktree/)).toBeNull();
  });

  it("tests every remote server in turn, one at a time", async () => {
    const pending: Array<(probe: HostProbe) => void> = [];
    testAsync.mockImplementation(() => new Promise<HostProbe>((resolve) => pending.push(resolve)));
    setup({ connection: status("connected", null), file: { hosts: [NEXUS, LAPTOP], active: null } });

    fireEvent.click(screen.getByRole("button", { name: "Test all" }));

    await waitFor(() => expect(testAsync).toHaveBeenCalledTimes(1));
    expect(testAsync).toHaveBeenLastCalledWith(NEXUS);
    await act(async () => pending[0]({ ok: true, serverVersion: 1, error: null }));
    await waitFor(() => expect(testAsync).toHaveBeenCalledTimes(2));
    expect(testAsync).toHaveBeenLastCalledWith(LAPTOP);
    await act(async () => pending[1]({ ok: true, serverVersion: 1, error: null }));
  });

  it("locks the default server's destination while editing it", () => {
    setup();

    fireEvent.click(within(row("Nexus")).getByRole("button", { name: "Edit" }));
    const editing = screen.getByLabelText("Destination for Nexus") as HTMLInputElement;
    expect(editing.disabled).toBe(true);
    expect(screen.queryByText(/fresh layout/)).toBeNull();
  });

  it("edits another server in place, warning that a new destination starts fresh", async () => {
    setup();

    fireEvent.click(within(row("Laptop")).getByRole("button", { name: "Edit" }));
    const field = screen.getByLabelText("Destination for Laptop") as HTMLInputElement;
    expect(field.disabled).toBe(false);
    expect(screen.getByText("Changing the destination starts this server with a fresh layout.")).toBeTruthy();

    type(field, "jt@laptop.lan");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
    });

    expect(saveAsync).toHaveBeenCalledWith({
      hosts: [NEXUS, { destination: "jt@laptop.lan", label: "Laptop" }],
      active: "nexus",
      localProfile: null,
    });
  });

  it("refuses an edit that would give a server an invalid destination", () => {
    setup();

    fireEvent.click(within(row("Laptop")).getByRole("button", { name: "Edit" }));
    type(screen.getByLabelText("Destination for Laptop") as HTMLInputElement, "http://laptop");

    expect(screen.getByRole("alert").textContent).toContain("not a URL");
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
