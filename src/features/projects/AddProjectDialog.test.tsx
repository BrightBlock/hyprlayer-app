// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { RepoPathStatus } from "@/lib/bindings";
import { AddProjectDialog } from "./AddProjectDialog";

// Mock the data hooks so the dialog runs its real logic against plain values —
// no react-query, no Tauri IPC. `vi.hoisted` lets the hoisted vi.mock factory
// reach them.
const q = vi.hoisted(() => ({
  useBootstrap: vi.fn(),
  useCloneProject: vi.fn(),
  useImportProject: vi.fn(),
  useDefaultUsername: vi.fn(),
  useRepoPathInfo: vi.fn(),
  useRecentRepos: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
// Identity debounce so the path probe resolves synchronously in the test.
vi.mock("@/lib/useDebounced", () => ({ useDebounced: (v: unknown) => v }));
vi.mock("@tauri-apps/api/path", () => ({
  homeDir: () => Promise.resolve("/home"),
  join: (...p: string[]) => Promise.resolve(p.join("/")),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

afterEach(cleanup);

function mutation() {
  return { mutate: vi.fn(), reset: vi.fn(), isPending: false, isError: false, error: null };
}

/** Render in disk mode with a mocked classification for the browsed folder. */
function setupDisk(status: RepoPathStatus | undefined) {
  const importProject = mutation();
  q.useBootstrap.mockReturnValue({
    data: { config: { state: "ok" }, settings: { reposRoot: "/repos" } },
  });
  q.useCloneProject.mockReturnValue(mutation());
  q.useImportProject.mockReturnValue(importProject);
  q.useDefaultUsername.mockReturnValue({ data: "jane" });
  q.useRepoPathInfo.mockReturnValue({ data: status });
  q.useRecentRepos.mockReturnValue({ data: [], isLoading: false });

  render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
  // Switch to the "Open from disk" tab and type a path.
  fireEvent.click(screen.getByRole("button", { name: "Open from disk" }));
  fireEvent.change(screen.getByPlaceholderText("/path/to/your/repo"), {
    target: { value: "/my/folder" },
  });
  return { importProject };
}

describe("AddProjectDialog disk import", () => {
  it("offers to initialize a folder that isn't a git repo", () => {
    setupDisk("nonGit");
    expect(screen.getByText(/Not a git repository/)).toBeDefined();
    expect(screen.getByRole("button", { name: /Initialize & add/ })).toBeDefined();
  });

  it("initializes with the chosen branch on submit", () => {
    const { importProject } = setupDisk("nonGit");
    // Override the default branch name.
    fireEvent.change(screen.getByPlaceholderText("main"), { target: { value: "master" } });
    fireEvent.click(screen.getByRole("button", { name: /Initialize & add/ }));
    expect(importProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ path: "/my/folder", branch: "master" }),
      expect.anything(),
    );
  });

  it("imports a ready repo in place, with no offer and no branch", () => {
    const { importProject } = setupDisk("ready");
    expect(screen.queryByText(/Not a git repository/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Add project" }));
    expect(importProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ path: "/my/folder", branch: null }),
      expect.anything(),
    );
  });

  it("refuses a bare repo and blocks submit", () => {
    const { importProject } = setupDisk("bare");
    expect(screen.getByText(/bare repository/i)).toBeDefined();
    // Submission is gated (canSubmit is false), so nothing is dispatched.
    fireEvent.click(screen.getByRole("button", { name: "Add project" }));
    expect(importProject.mutate).not.toHaveBeenCalled();
  });

  it("still routes to import before the probe resolves (no stale mis-routing)", () => {
    // With the probe unresolved (data undefined) we can't yet know the folder is
    // non-git, but the single import command re-classifies on the backend — so an
    // early submit dispatches import rather than erroring with `not_a_git_repo`.
    const { importProject } = setupDisk(undefined);
    expect(screen.queryByText(/Not a git repository/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Add project" }));
    expect(importProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ path: "/my/folder", branch: null }),
      expect.anything(),
    );
  });
});
