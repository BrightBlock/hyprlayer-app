// @vitest-environment jsdom
import { QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ArchiveInfo, ArchivedWorkstreamView, Bootstrap } from "@/lib/bindings";
import { queryClient } from "@/lib/queryClient";

const b = vi.hoisted(() => ({
  getBootstrap: vi.fn(),
  connectionStatus: vi.fn(),
  listArchivedWorkstreams: vi.fn(),
  refreshArchivedWorkstreams: vi.fn(),
  keepArchivedWorkstream: vi.fn(),
  deleteArchivedWorkstream: vi.fn(),
  purgeArchivedWorkstreams: vi.fn(),
  restoreWorkstream: vi.fn(),
  listSessions: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({ commands: b }));
vi.mock("@/lib/log", () => ({ log: { warn: vi.fn(), info: vi.fn(), error: vi.fn() } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { ArchivedWorktreesPopover } from "./ArchivedWorktreesPopover";
import {
  closeArchivedWorktrees,
  openArchivedWorktrees,
  useArchivedWorktrees,
} from "./archivedWorktreesStore";

function ArchivedWorktreesButton() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useArchivedWorktrees((s) => s.open);
  return (
    <>
      <button ref={anchorRef} type="button" onClick={openArchivedWorktrees}>
        Archived worktrees
      </button>
      <ArchivedWorktreesPopover open={open} onClose={closeArchivedWorktrees} anchorRef={anchorRef} />
    </>
  );
}

const DAY = 86_400_000;
const NOW = Date.UTC(2026, 8, 24, 12, 0, 0);
const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const fail = (code: string) =>
  Promise.resolve({ status: "error" as const, error: { code, message: code, remedy: null } });

const BOOTSTRAP = {
  projects: [
    { path: "/code/acme-web", mappedName: "acme-web" },
    { path: "/code/billing-api", mappedName: "billing-api" },
  ],
} as unknown as Bootstrap;

function archive(
  branch: string,
  project: string,
  age: number,
  over: Partial<ArchiveInfo> = {},
): ArchivedWorkstreamView {
  return {
    item: {
      id: `id-${branch}`,
      projectPath: `/code/${project}`,
      title: branch,
      slug: branch,
      branch,
      worktreePath: `/wt/${branch}`,
      baseBranch: "main",
      worktreeMode: "isolated",
      stage: "general",
      status: "archived",
      prNumber: null,
      createdAt: 1,
      updatedAt: 2,
      archivedAt: NOW - age * DAY,
    },
    archivePath: `/code/${project}/archived-contexts/${branch}`,
    archiveExists: true,
    info: {
      purgeInMs: (30 - age) * DAY,
      kept: false,
      sizeBytes: 100e6,
      uncommitted: 0,
      unpushed: 0,
      deletesBranch: true,
      pr: { state: "none" },
      ...over,
    },
  };
}

const merged = archive("og-image-generator", "acme-web", 3, {
  sizeBytes: 212e6,
  pr: { state: "merged", number: 38 },
});
const dirty = archive("fix-login", "billing-api", 10, { sizeBytes: 1.2e9, uncommitted: 2 });
const expiring = archive("old-spike", "acme-web", 26, { sizeBytes: 40e6, uncommitted: 1 });

let listed: ArchivedWorkstreamView[] = [];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  listed = [merged, dirty, expiring];
  b.getBootstrap.mockImplementation(() => ok(BOOTSTRAP));
  b.connectionStatus.mockImplementation(() => ok({ state: "connected", host: null }));
  b.listArchivedWorkstreams.mockImplementation(() => ok(listed));
  b.refreshArchivedWorkstreams.mockImplementation(() => ok(0));
  b.keepArchivedWorkstream.mockImplementation(() => ok(null));
  b.deleteArchivedWorkstream.mockImplementation(() => ok(null));
  b.purgeArchivedWorkstreams.mockImplementation(() => ok({ deleted: 1, failed: 0 }));
  b.restoreWorkstream.mockImplementation(() =>
    ok({ item: { ...merged.item, status: "active" }, git: {}, seed: null }),
  );
  b.listSessions.mockImplementation(() => ok([]));
});

afterEach(() => {
  act(() => closeArchivedWorktrees());
  cleanup();
  queryClient.clear();
  vi.useRealTimers();
  vi.clearAllMocks();
});

async function openPopover() {
  render(
    <QueryClientProvider client={queryClient}>
      <ArchivedWorktreesButton />
    </QueryClientProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Archived worktrees" }));
  const dialog = await screen.findByRole("dialog", { name: "Archived worktrees" });
  await within(dialog).findAllByRole("listitem");
  return dialog;
}

const rowFor = (branch: string) => screen.getByRole("listitem", { name: new RegExp(`^${branch},`) });
const names = () => screen.queryAllByRole("listitem").map((li) => li.getAttribute("aria-label")?.split(",")[0]);
const filterBox = () => screen.getByRole("textbox", { name: "Filter archived worktrees" });

describe("ArchivedWorktreesPopover", () => {
  it("opens as a labelled dialog over the page and refreshes PR state once", async () => {
    await openPopover();
    expect(screen.getByText("3 · 1.5 GB")).toBeTruthy();
    expect(b.refreshArchivedWorkstreams).toHaveBeenCalledTimes(1);
  });

  it("lists archives newest first with a tag and a meta line", async () => {
    await openPopover();
    expect(names()).toEqual(["og-image-generator", "fix-login", "old-spike"]);
    const row = rowFor("og-image-generator");
    expect(within(row).getByText("MERGED")).toBeTruthy();
    expect(within(row).getByText("acme-web · PR #38 · 3d · 212 MB")).toBeTruthy();
    expect(within(rowFor("fix-login")).getByText("ABANDONED")).toBeTruthy();
  });

  it("marks an archive in its last week and always offers Keep on it", async () => {
    await openPopover();
    const row = rowFor("old-spike");
    expect(within(row).getByText("PURGES IN 4d")).toBeTruthy();
    expect(within(row).getByText("acme-web · uncommitted work · 3w · 40 MB")).toBeTruthy();
    expect(row.className).toContain("opacity-75");
    expect(within(row).getByRole("button", { name: "Keep old-spike" })).toBeTruthy();
    expect(within(rowFor("fix-login")).queryByRole("button", { name: /^Keep/ })).toBeNull();
  });

  it("Keep pins the archive: the countdown goes and the row returns to full opacity", async () => {
    await openPopover();
    listed = [merged, dirty, { ...expiring, info: { ...expiring.info, kept: true } }];
    fireEvent.click(within(rowFor("old-spike")).getByRole("button", { name: "Keep old-spike" }));
    await waitFor(() => expect(b.keepArchivedWorkstream).toHaveBeenCalledWith("id-old-spike"));
    await waitFor(() => expect(within(rowFor("old-spike")).queryByText("PURGES IN 4d")).toBeNull());
    const row = rowFor("old-spike");
    expect(within(row).getByText("ABANDONED")).toBeTruthy();
    expect(row.className).not.toContain("opacity-75");
    expect(within(row).queryByRole("button", { name: "Keep old-spike" })).toBeNull();
  });

  it("warns before deleting an archive with uncommitted work", async () => {
    await openPopover();
    fireEvent.click(within(rowFor("fix-login")).getByRole("button", { name: "Delete fix-login" }));
    const confirm = await screen.findByRole("dialog", { name: "Delete fix-login?" });
    expect(within(confirm).getByText(/1\.2 GB/)).toBeTruthy();
    expect(within(confirm).getByText(/2 files with uncommitted changes/)).toBeTruthy();
    fireEvent.click(within(confirm).getByRole("button", { name: "Delete anyway" }));
    await waitFor(() => expect(b.deleteArchivedWorkstream).toHaveBeenCalledWith("id-fix-login"));
  });

  it("asks plainly when nothing would be lost", async () => {
    await openPopover();
    fireEvent.click(
      within(rowFor("og-image-generator")).getByRole("button", { name: "Delete og-image-generator" }),
    );
    const confirm = await screen.findByRole("dialog", { name: "Delete og-image-generator?" });
    expect(within(confirm).getByText(/212 MB/)).toBeTruthy();
    expect(within(confirm).queryByText(/uncommitted/)).toBeNull();
    expect(within(confirm).getByRole("button", { name: "Delete" })).toBeTruthy();
  });

  it("owns up when an archive could not be checked", async () => {
    listed = [archive("unscanned", "acme-web", 5, { uncommitted: null, unpushed: null })];
    await openPopover();
    fireEvent.click(within(rowFor("unscanned")).getByRole("button", { name: "Delete unscanned" }));
    const confirm = await screen.findByRole("dialog", { name: "Delete unscanned?" });
    expect(within(confirm).getByText(/couldn't be checked/)).toBeTruthy();
    expect(within(confirm).getByRole("button", { name: "Delete anyway" })).toBeTruthy();
  });

  it("filters live on branch and project, and Merged narrows to merged", async () => {
    await openPopover();
    fireEvent.change(filterBox(), { target: { value: "billing" } });
    expect(names()).toEqual(["fix-login"]);
    fireEvent.change(filterBox(), { target: { value: "SPIKE" } });
    expect(names()).toEqual(["old-spike"]);
    fireEvent.change(filterBox(), { target: { value: "" } });
    const toggle = screen.getByRole("button", { name: "Merged" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    expect(names()).toEqual(["og-image-generator"]);
  });

  it("offers Clear when a filter matches nothing", async () => {
    await openPopover();
    fireEvent.change(filterBox(), { target: { value: "zzz" } });
    expect(screen.getByText('No matches for "zzz"')).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(names()).toHaveLength(3);
  });

  it("says so when nothing is archived, without a footer", async () => {
    listed = [];
    render(
      <QueryClientProvider client={queryClient}>
        <ArchivedWorktreesButton />
      </QueryClientProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Archived worktrees" }));
    expect(await screen.findByText("No archived worktrees.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Purge merged" })).toBeNull();
  });

  it("moves through rows with the arrows and restores on Enter", async () => {
    await openPopover();
    fireEvent.keyDown(filterBox(), { key: "ArrowDown" });
    expect(document.activeElement).toBe(rowFor("og-image-generator"));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement).toBe(rowFor("fix-login"));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    expect(document.activeElement).toBe(rowFor("og-image-generator"));
    fireEvent.keyDown(document.activeElement!, { key: "Enter" });
    await waitFor(() => expect(b.restoreWorkstream).toHaveBeenCalledWith("id-og-image-generator", null));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Archived worktrees" })).toBeNull());
  });

  it("asks to delete on Delete or Backspace, and Cancel hands focus back to the row", async () => {
    await openPopover();
    rowFor("fix-login").focus();
    fireEvent.keyDown(rowFor("fix-login"), { key: "Backspace" });
    const confirm = await screen.findByRole("dialog", { name: "Delete fix-login?" });
    fireEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(document.activeElement).toBe(rowFor("fix-login")));
    fireEvent.keyDown(rowFor("fix-login"), { key: "Delete" });
    expect(await screen.findByRole("dialog", { name: "Delete fix-login?" })).toBeTruthy();
    expect(b.deleteArchivedWorkstream).not.toHaveBeenCalled();
  });

  it("asks where to restore when the original path is taken", async () => {
    b.restoreWorkstream.mockImplementationOnce(() => fail("restore_target_occupied"));
    await openPopover();
    fireEvent.click(
      within(rowFor("og-image-generator")).getByRole("button", { name: "Restore og-image-generator" }),
    );
    const ask = await screen.findByRole("dialog", { name: "Restore og-image-generator elsewhere" });
    const path = within(ask).getByRole("textbox") as HTMLInputElement;
    expect(path.value).toBe("/wt/og-image-generator-restored");
    fireEvent.change(path, { target: { value: "/elsewhere" } });
    fireEvent.click(within(ask).getByRole("button", { name: "Restore" }));
    await waitFor(() =>
      expect(b.restoreWorkstream).toHaveBeenLastCalledWith("id-og-image-generator", "/elsewhere"),
    );
  });

  it("purges merged archives, leaving kept ones, after confirming the count and size", async () => {
    const keptMerged = archive("kept-merged", "acme-web", 5, {
      kept: true,
      pr: { state: "merged", number: 2 },
    });
    listed = [merged, dirty, keptMerged];
    await openPopover();
    fireEvent.click(screen.getByRole("button", { name: "Purge merged" }));
    const confirm = await screen.findByRole("dialog", { name: "Delete 1 merged worktree (212 MB)?" });
    fireEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(b.purgeArchivedWorkstreams).toHaveBeenCalledWith(["id-og-image-generator"]),
    );
  });

  it("disables Purge merged when nothing qualifies", async () => {
    listed = [dirty, expiring];
    await openPopover();
    expect((screen.getByRole("button", { name: "Purge merged" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
