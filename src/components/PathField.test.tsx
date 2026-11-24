// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DirListing } from "@/lib/bindings";
import { PathField } from "./PathField";

const m = vi.hoisted(() => ({
  useConnectionStatus: vi.fn(),
  browseDirectory: vi.fn(),
  open: vi.fn(),
}));
vi.mock("@/lib/queries", () => ({ useConnectionStatus: m.useConnectionStatus }));
vi.mock("@/lib/bindings", () => ({ commands: { browseDirectory: m.browseDirectory } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: m.open }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const NEXUS = { destination: "nexus", label: "Nexus" };

function listing(path: string, parent: string | null, names: string[]): DirListing {
  return {
    path,
    parent,
    entries: names.map((name) => ({
      name,
      isDir: !name.includes("."),
      isRepo: name === "app",
      path: `${path}/${name}`,
    })),
  };
}

const pathInput = () => screen.getByPlaceholderText(/Type a path/) as HTMLInputElement;

function setup(host: typeof NEXUS | null) {
  m.useConnectionStatus.mockReturnValue({ data: { host } });
  const onChange = vi.fn();
  render(<PathField value="" onChange={onChange} browseTitle="Choose a folder" />);
  return { onChange };
}

describe("PathField", () => {
  it("opens the native dialog when the server is local", async () => {
    const { onChange } = setup(null);
    m.open.mockResolvedValue("/home/jane/repos");
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("/home/jane/repos"));
    expect(m.browseDirectory).not.toHaveBeenCalled();
  });

  it("opens the server-side picker when the server is remote", async () => {
    setup(NEXUS);
    m.browseDirectory.mockResolvedValue({ status: "ok", data: listing("/srv", null, ["app"]) });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    expect(await screen.findByText("app")).toBeTruthy();
    expect(screen.getByText("repo")).toBeTruthy();
    expect(m.browseDirectory).toHaveBeenCalledWith(null);
    expect(m.open).not.toHaveBeenCalled();
  });

  it("navigates with the entry's own server-joined path", async () => {
    setup(NEXUS);
    m.browseDirectory.mockResolvedValue({ status: "ok", data: listing("/srv", null, ["work"]) });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    fireEvent.click(await screen.findByText("work"));
    await waitFor(() => expect(m.browseDirectory).toHaveBeenLastCalledWith("/srv/work"));
  });

  it("chooses the directory being listed, not a selected entry", async () => {
    const { onChange } = setup(NEXUS);
    m.browseDirectory.mockResolvedValue({
      status: "ok",
      data: listing("/srv/work", "/srv", ["a.txt"]),
    });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    // The listed path lives in the picker's own path input, not a label.
    await waitFor(() =>
      expect(pathInput().value).toBe("/srv/work/"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose this folder" }));
    expect(onChange).toHaveBeenCalledWith("/srv/work");
  });

  it("filters the current folder without re-querying the server", async () => {
    setup(NEXUS);
    m.browseDirectory.mockResolvedValue({
      status: "ok",
      data: listing("/srv", null, ["work", "warehouse", "logs"]),
    });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    await screen.findByText("work");
    await waitFor(() => expect(pathInput().value).toBe("/srv/"));
    m.browseDirectory.mockClear();

    fireEvent.change(pathInput(), { target: { value: "/srv/wa" } });

    await waitFor(() => expect(screen.queryByText("logs")).toBeNull());
    // The surviving row keeps its whole name; "wa" is merely highlighted in it.
    expect(screen.getByRole("button", { name: "warehouse" })).toBeTruthy();
    // "rehouse" renders twice — once in the row, once as the inline completion.
    expect(screen.getAllByText("rehouse")).toHaveLength(2);
    // The directory did not change, so no second round trip.
    expect(m.browseDirectory).not.toHaveBeenCalled();
  });

  it("chooses a fully typed directory, not its parent", async () => {
    const { onChange } = setup(NEXUS);
    m.browseDirectory.mockResolvedValue({
      status: "ok",
      data: listing("/srv", null, ["work", "logs"]),
    });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    await screen.findByText("work");

    // The trailing segment names a real directory, so it is the target — the
    // listed directory /srv is only the fallback.
    fireEvent.change(pathInput(), { target: { value: "/srv/work" } });
    fireEvent.click(screen.getByRole("button", { name: "Choose this folder" }));
    expect(onChange).toHaveBeenCalledWith("/srv/work");
  });

  it("caps how many rows it renders and says how many it held back", async () => {
    setup(NEXUS);
    const many = Array.from({ length: 260 }, (_, i) => `dir${String(i).padStart(3, "0")}`);
    m.browseDirectory.mockResolvedValue({ status: "ok", data: listing("/srv", null, many) });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    await screen.findByText("dir000");

    // 200 rendered, not 260 — the rest are behind the filter, not the scrollbar.
    expect(screen.queryByText("dir199")).toBeTruthy();
    expect(screen.queryByText("dir200")).toBeNull();
    expect(screen.getByText(/60 more — keep typing/)).toBeTruthy();
  });

  it("hides dot files until asked", async () => {
    setup(NEXUS);
    m.browseDirectory.mockResolvedValue({
      status: "ok",
      data: listing("/srv", null, [".cache", "work"]),
    });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    await screen.findByText("work");
    expect(screen.queryByText(".cache")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Dotfiles" }));
    await screen.findByText(".cache");
  });

  it("walks up with the server's own parent, and stops at the top", async () => {
    // The server supplies the parent, so no separator is guessed client-side.
    setup(NEXUS);
    m.browseDirectory.mockResolvedValue({ status: "ok", data: listing("/home/jt/projects", "/home/jt", ["app"]) });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));

    // The button renders before the first listing resolves, so wait for the
    // parent to arrive rather than for the element.
    const up = await vi.waitFor(() => {
      const b = screen.getByRole("button", { name: "Up one folder" }) as HTMLButtonElement;
      if (b.disabled) throw new Error("listing not loaded yet");
      return b;
    });

    m.browseDirectory.mockResolvedValue({ status: "ok", data: listing("/home/jt", "/home", ["projects"]) });
    fireEvent.click(up);
    await vi.waitFor(() => expect(m.browseDirectory).toHaveBeenCalledWith("/home/jt"));
  });

  it("disables Up at the filesystem root, where the server reports no parent", async () => {
    setup(NEXUS);
    m.browseDirectory.mockResolvedValue({ status: "ok", data: listing("/", null, ["home"]) });
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));

    const up = await screen.findByRole("button", { name: "Up one folder" });
    expect((up as HTMLButtonElement).disabled).toBe(true);
  });
});
