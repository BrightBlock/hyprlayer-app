// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Host, HostsFile, ProvisionStep } from "@/lib/bindings";
import { HostList } from "./HostList";

const q = vi.hoisted(() => ({ useTestHostConnection: vi.fn(), useConnectionStatus: vi.fn() }));
vi.mock("@/lib/queries", () => q);

// Only the IPC call is stubbed; the step labels and the failure code stay real,
// so a rename in that module fails these rather than passing against a copy.
const install = vi.hoisted(() => ({ runProvision: vi.fn() }));
vi.mock("./serverInstall", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./serverInstall")>()),
  runProvision: install.runProvision,
}));

const reconnect = vi.hoisted(() => ({ retryConnection: vi.fn() }));
vi.mock("./reconnect", () => reconnect);

const mutateAsync = vi.fn();

const NO_SERVER_PROBE = {
  ok: false,
  serverVersion: null,
  error: { code: "remote_cli_missing", message: "hyprlayer-server is not installed on nexus." },
};

beforeEach(() => {
  q.useTestHostConnection.mockReturnValue({ mutateAsync });
  // Not connected by default: the status column then reflects probes alone.
  q.useConnectionStatus.mockReturnValue({ data: { state: "failed", host: null } });
  install.runProvision.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const NEXUS: Host = { destination: "nexus", label: "Nexus" };
const LAPTOP: Host = { destination: "jt@laptop.local", label: "Laptop" };

function file(hosts: Host[], active: string | null = null): HostsFile {
  return { hosts, active };
}

function renderList(value: HostsFile = file([])) {
  const onChange = vi.fn();
  const onSelect = vi.fn();
  const view = render(<HostList value={value} onChange={onChange} onSelect={onSelect} />);
  return { onChange, onSelect, view };
}

const labels = () => screen.getAllByLabelText("Label") as HTMLInputElement[];
const destinations = () => screen.getAllByLabelText("Destination") as HTMLInputElement[];
// getAllBy* throws when nothing matches, so absence needs the query variant.
const editingRows = () => screen.queryAllByLabelText("Destination");
const addHost = () => screen.getByRole("button", { name: "Add host" });
const confirmAdd = () => screen.getByRole("button", { name: "Add" }) as HTMLButtonElement;

function type(input: HTMLInputElement, text: string) {
  fireEvent.change(input, { target: { value: text } });
}

function openRowMenu(name: string) {
  fireEvent.click(screen.getByRole("button", { name: `More actions for ${name}` }));
}

describe("HostList", () => {
  it("says the app runs on this machine when no host is configured", () => {
    renderList();
    expect(screen.getByText(/No hosts yet/)).toBeTruthy();
    expect(screen.queryByLabelText("Destination")).toBeNull();
  });

  it("refuses to add a host with a blank destination", () => {
    const { onChange } = renderList();
    fireEvent.click(addHost());
    type(labels()[0], "Nexus");

    expect(confirmAdd().disabled).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();

    fireEvent.click(confirmAdd());
    expect(onChange).not.toHaveBeenCalled();
  });

  it("refuses a destination that duplicates an existing host's", () => {
    const { onChange } = renderList(file([NEXUS]));
    fireEvent.click(addHost());
    type(labels()[0], "Nexus again");
    type(destinations()[0], "nexus");

    expect(confirmAdd().disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toMatch(/already uses/);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("refuses `local`, the name that belongs to this machine's profile", () => {
    const { onChange } = renderList();
    fireEvent.click(addHost());
    type(labels()[0], "Local");
    type(destinations()[0], "local");

    expect(confirmAdd().disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain('cannot be "local"');
    expect(onChange).not.toHaveBeenCalled();
  });

  it("refuses a destination with whitespace in it", () => {
    renderList();
    fireEvent.click(addHost());
    type(labels()[0], "Nexus");
    type(destinations()[0], "jt@nexus local");

    expect(confirmAdd().disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("cannot contain spaces");
  });

  it("adds a valid host and commits the whole file", () => {
    const { onChange } = renderList(file([NEXUS], "nexus"));
    fireEvent.click(addHost());
    type(labels()[0], "Laptop");
    type(destinations()[0], "jt@laptop.local");
    fireEvent.click(confirmAdd());

    expect(onChange).toHaveBeenCalledWith({ hosts: [NEXUS, LAPTOP], active: "nexus", localProfile: null });
  });

  it("does not clobber an in-flight edit when a refetch delivers the same file", () => {
    const value = file([NEXUS, LAPTOP]);
    const onChange = vi.fn();
    const { rerender } = render(<HostList value={value} onChange={onChange} onSelect={vi.fn()} />);

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }));
    type(destinations()[0], "nexus.local");

    // What an invalidated ["hosts"] query hands back: a new object, same bytes.
    rerender(<HostList value={file([NEXUS, LAPTOP])} onChange={onChange} onSelect={vi.fn()} />);

    expect(destinations()[0].value).toBe("nexus.local");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("removes a row and commits immediately", () => {
    const { onChange } = renderList(file([NEXUS, LAPTOP], "jt@laptop.local"));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Remove" }));

    expect(onChange).toHaveBeenCalledWith({ hosts: [LAPTOP], active: "jt@laptop.local", localProfile: null });
  });

  it("falls back to this machine when the active host is the one removed", () => {
    const { onChange } = renderList(file([NEXUS, LAPTOP], "nexus"));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Remove" }));

    expect(onChange).toHaveBeenCalledWith({ hosts: [LAPTOP], active: null, localProfile: null });
  });

  // Reordering runs on pointer events, so the rows need real geometry: jsdom
  // gives every element a zero-sized rect, and `rowIndexAt` reads rects.
  function stubRowGeometry(height = 40) {
    screen.getAllByRole("listitem").forEach((li, i) => {
      li.getBoundingClientRect = () =>
        ({ top: i * height, bottom: (i + 1) * height, height }) as DOMRect;
    });
  }

  function dragRow(from: number, toY: number) {
    const row = screen.getAllByRole("listitem")[from].firstElementChild as HTMLElement;
    row.setPointerCapture = () => {};
    row.hasPointerCapture = () => false;
    row.releasePointerCapture = () => {};
    fireEvent.pointerDown(row, { button: 0, pointerId: 1, clientY: from * 40 + 20 });
    fireEvent.pointerMove(row, { pointerId: 1, clientY: toY });
    fireEvent.pointerUp(row, { pointerId: 1, clientY: toY });
  }

  it("reorders by dragging a row onto another and commits the new order", () => {
    const { onChange } = renderList(file([NEXUS, LAPTOP]));
    stubRowGeometry();

    dragRow(0, 60); // into the second row's band

    expect(onChange).toHaveBeenCalledWith({ hosts: [LAPTOP, NEXUS], active: null, localProfile: null });
  });

  it("treats a press that never moves as a click, not a reorder", () => {
    const { onChange } = renderList(file([NEXUS, LAPTOP]));
    stubRowGeometry();

    dragRow(0, 21); // one pixel: under the drag threshold

    expect(onChange).not.toHaveBeenCalled();
  });

  it("does not start a reorder from a press on a row control", () => {
    const { onChange } = renderList(file([NEXUS, LAPTOP]));
    stubRowGeometry();
    const row = screen.getAllByRole("listitem")[0].firstElementChild as HTMLElement;
    row.setPointerCapture = () => {};
    row.hasPointerCapture = () => false;
    row.releasePointerCapture = () => {};

    fireEvent.pointerDown(screen.getByRole("button", { name: "Test Nexus" }), {
      button: 0,
      pointerId: 1,
      clientY: 20,
    });
    fireEvent.pointerMove(row, { pointerId: 1, clientY: 60 });
    fireEvent.pointerUp(row, { pointerId: 1, clientY: 60 });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("hands a clicked row up as a switch, not an edit, and marks the active one", () => {
    const { onChange, onSelect } = renderList(file([NEXUS, LAPTOP], "nexus"));

    // The row itself is the control; the active one is inert and aria-current.
    const activeRow = screen.getByTitle("Nexus is the active server") as HTMLButtonElement;
    expect(activeRow.getAttribute("aria-current")).toBe("true");
    expect(activeRow.disabled).toBe(true);

    fireEvent.click(screen.getByTitle("Use Laptop"));

    // A switch is confirmed and relaunched by the caller; the list writes nothing.
    expect(onSelect).toHaveBeenCalledWith({
      hosts: [NEXUS, LAPTOP],
      active: "jt@laptop.local",
      localProfile: null,
    });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("selects on a real press-and-release, not just a synthetic click", () => {
    const { onSelect } = renderList(file([NEXUS, LAPTOP], "nexus"));
    stubRowGeometry();
    const row = screen.getAllByRole("listitem")[1].firstElementChild as HTMLElement;
    const captured: number[] = [];
    row.setPointerCapture = (id: number) => void captured.push(id);
    row.hasPointerCapture = () => false;
    row.releasePointerCapture = () => {};
    const select = screen.getByTitle("Use Laptop");

    // What a mouse actually does. The row must not capture the pointer here,
    // or the click is retargeted to the row and the button never sees it.
    fireEvent.pointerDown(select, { button: 0, pointerId: 1, clientY: 60 });
    fireEvent.pointerUp(select, { pointerId: 1, clientY: 60 });
    fireEvent.click(select);

    expect(captured).toEqual([]);
    expect(onSelect).toHaveBeenCalledWith({
      hosts: [NEXUS, LAPTOP],
      active: "jt@laptop.local",
      localProfile: null,
    });
  });

  it("does not select the row a reorder was dragged from", () => {
    const { onSelect } = renderList(file([NEXUS, LAPTOP]));
    stubRowGeometry();

    dragRow(0, 60);
    // The drop lands as a click on the row that was dragged.
    fireEvent.click(screen.getByTitle("Use Nexus"));

    expect(onSelect).not.toHaveBeenCalled();
  });

  it("commits an edited row on blur", () => {
    const { onChange } = renderList(file([NEXUS]));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }));
    type(destinations()[0], "jt@nexus.local");
    fireEvent.blur(destinations()[0]);

    expect(onChange).toHaveBeenCalledWith({
      hosts: [{ destination: "jt@nexus.local", label: "Nexus" }],
      active: null,
      localProfile: null,
    });
  });

  it("keeps a blanked row in edit mode rather than committing it", () => {
    const { onChange } = renderList(file([NEXUS]));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }));
    type(destinations()[0], "");
    fireEvent.blur(destinations()[0]);

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("SSH destination required");
    expect(destinations()).toHaveLength(1);
  });

  it("escapes out of a row that has been blanked, restoring what was saved", () => {
    const { onChange } = renderList(file([NEXUS]));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }));
    type(destinations()[0], "");
    // Without a way out this row is a trap: it will not commit and Enter is
    // refused while it is invalid.
    fireEvent.keyDown(destinations()[0], { key: "Escape" });

    expect(editingRows()).toHaveLength(0);
    expect(screen.getByText("nexus")).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("cancels an edit with the row's own button, keeping the saved value", () => {
    const { onChange } = renderList(file([NEXUS]));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }));
    type(destinations()[0], "jt@somewhere.else");
    fireEvent.click(screen.getByRole("button", { name: "Cancel editing Nexus" }));

    expect(editingRows()).toHaveLength(0);
    expect(screen.getByText("nexus")).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("commits and leaves edit mode from the row's Done button", () => {
    const { onChange } = renderList(file([NEXUS]));

    openRowMenu("Nexus");
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit" }));
    type(destinations()[0], "jt@nexus.local");
    fireEvent.click(screen.getByRole("button", { name: "Done editing Nexus" }));

    expect(onChange).toHaveBeenCalledWith({
      hosts: [{ destination: "jt@nexus.local", label: "Nexus" }],
      active: null,
      localProfile: null,
    });
    expect(editingRows()).toHaveLength(0);
  });

  it("reports a passing test in the status column", async () => {
    mutateAsync.mockResolvedValue({ ok: true, serverVersion: 3, error: null });
    renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));

    // The outcome, not a version number to decode.
    expect(await screen.findByText("Answered")).toBeTruthy();
    expect(mutateAsync).toHaveBeenCalledWith(NEXUS);
  });

  it("summarises a failure in the column and keeps the ssh text as the tooltip", async () => {
    mutateAsync.mockResolvedValue(NO_SERVER_PROBE);
    renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));

    const short = await screen.findByText("No server");
    expect(short).toBeTruthy();
    // The detail is not lost, just not setting the width of the list.
    expect(short.closest("[title]")?.getAttribute("title")).toContain(
      "hyprlayer-server is not installed on nexus.",
    );
  });

  it("falls back to a plain label for a failure code it does not know", async () => {
    mutateAsync.mockResolvedValue({
      ok: false,
      serverVersion: null,
      error: { code: "something_new", message: "Whatever ssh said." },
    });
    renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));

    expect(await screen.findByText("Failed")).toBeTruthy();
  });

  it("offers an install only on the failure a host can be provisioned out of", async () => {
    mutateAsync.mockResolvedValue({
      ok: false,
      serverVersion: null,
      error: { code: "remote_auth_failed", message: "Permission denied (publickey)." },
    });
    renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));

    expect(await screen.findByText("Auth failed")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("installs the server from the row, without switching to the host", async () => {
    mutateAsync
      .mockResolvedValueOnce(NO_SERVER_PROBE)
      .mockResolvedValueOnce({ ok: true, serverVersion: 3, error: null });
    const { onChange, onSelect } = renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));
    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() =>
      expect(install.runProvision).toHaveBeenCalledWith(NEXUS, expect.any(Function)),
    );
    // Preparing a host is not choosing it: nothing is written and nothing
    // restarts, which is the whole point of doing it from here.
    expect(onSelect).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
    // The re-probe, not the install, is what says the host is ready now.
    expect(await screen.findByText("Answered")).toBeTruthy();
  });

  it("carries the live step in the row's own status column", async () => {
    mutateAsync.mockResolvedValue(NO_SERVER_PROBE);
    let emit!: (step: ProvisionStep) => void;
    // Never settles, so the row stays mid-install for the assertions.
    install.runProvision.mockImplementation((_host: Host, onStep: (s: ProvisionStep) => void) => {
      emit = onStep;
      return new Promise<void>(() => {});
    });
    renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));
    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    expect(await screen.findByText(/Checking the host/)).toBeTruthy();
    act(() => emit("upload"));
    expect(await screen.findByText(/Copying to the host/)).toBeTruthy();
    // A second push cannot start while one is in flight.
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
    expect((screen.getByRole("button", { name: "Test Nexus" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("keeps the install offered when the push fails, and the ssh text as the tooltip", async () => {
    mutateAsync.mockResolvedValue(NO_SERVER_PROBE);
    install.runProvision.mockRejectedValue(new Error("ssh: connection closed by remote host"));
    renderList(file([NEXUS]));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));
    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    const short = await screen.findByText("Install failed");
    expect(short.closest("[title]")?.getAttribute("title")).toContain("ssh: connection closed");
    // The server is as missing as it was, so retrying stays one click.
    expect(screen.getByRole("button", { name: "Install" })).toBeTruthy();
  });

  it("reconnects when the active host answers, so the header stops contradicting the row", async () => {
    mutateAsync.mockResolvedValue({ ok: true, serverVersion: 3, error: null });
    renderList(file([NEXUS], "nexus"));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));

    expect(await screen.findByText("Answered")).toBeTruthy();
    await waitFor(() => expect(reconnect.retryConnection).toHaveBeenCalledTimes(1));
  });

  it("leaves the connection alone when the host that answered is not the active one", async () => {
    mutateAsync.mockResolvedValue({ ok: true, serverVersion: 3, error: null });
    renderList(file([NEXUS, LAPTOP], "nexus"));

    fireEvent.click(screen.getByRole("button", { name: "Test Laptop" }));

    // Laptop answering says nothing about the connection to Nexus.
    expect(await screen.findByText("Answered")).toBeTruthy();
    expect(reconnect.retryConnection).not.toHaveBeenCalled();
  });

  it("reconnects after installing onto the active host, via the probe that follows", async () => {
    mutateAsync
      .mockResolvedValueOnce(NO_SERVER_PROBE)
      .mockResolvedValueOnce({ ok: true, serverVersion: 3, error: null });
    renderList(file([NEXUS], "nexus"));

    fireEvent.click(screen.getByRole("button", { name: "Test Nexus" }));
    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    expect(await screen.findByText("Answered")).toBeTruthy();
    await waitFor(() => expect(reconnect.retryConnection).toHaveBeenCalledTimes(1));
  });

  it("leaves the live connection to the dialog header, marking the active row only", () => {
    q.useConnectionStatus.mockReturnValue({
      data: { state: "connected", host: { destination: "nexus", label: "Nexus" } },
    });
    renderList(file([NEXUS, LAPTOP], "nexus"));

    // The status slot carries probe results; a live connection is the header's
    // one statement of that fact, and the row is already marked active by its
    // check and tint. Repeating it here said the same thing a third time.
    expect(screen.queryByText("Connected")).toBeNull();
    expect(screen.queryByText("Answered")).toBeNull();
    expect(screen.getByTitle("Nexus is the active server")).toBeTruthy();
  });
});
