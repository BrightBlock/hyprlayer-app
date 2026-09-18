// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChangesPanel } from "./ChangesPanel";

const q = vi.hoisted(() => ({
  useWorktreeGitStatus: vi.fn(),
  useWorktreeDir: vi.fn(),
  useStageFiles: vi.fn(),
  useUnstageFiles: vi.fn(),
  useDiscardFiles: vi.fn(),
  useWorkstreamCommits: vi.fn(),
  useCommitFiles: vi.fn(),
  useProjectMemory: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
vi.mock("@/lib/queryClient", () => ({ queryClient: { invalidateQueries: vi.fn() } }));
vi.mock("@/features/session/sessionStore", () => ({
  useWorkstreamDocs: () => [],
  useWorkstreamArtifacts: () => [],
}));

afterEach(cleanup);

function setup(filesOnly: boolean) {
  const mutation = () => ({ mutate: vi.fn(), isPending: false });
  q.useWorktreeGitStatus.mockReturnValue({
    data: { staged: [], unstaged: [] },
    isLoading: false,
    isError: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  });
  q.useWorktreeDir.mockReturnValue({
    data: [{ name: "todo.md", path: "todo.md", isDir: false }],
    isLoading: false,
    isError: false,
    error: null,
  });
  q.useStageFiles.mockReturnValue(mutation());
  q.useUnstageFiles.mockReturnValue(mutation());
  q.useDiscardFiles.mockReturnValue(mutation());
  q.useWorkstreamCommits.mockReturnValue({
    data: [],
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  });
  q.useProjectMemory.mockReturnValue({
    data: { dir: "/c/projects/-r/memory", keyCheck: "verified", files: [] },
    isLoading: false,
    isError: false,
    error: null,
  });

  render(
    <ChangesPanel
      workstreamId="ws1"
      onOpenDoc={vi.fn()}
      onOpenDiff={vi.fn()}
      onOpenFile={vi.fn()}
      filesOnly={filesOnly}
      baseBranch="main"
    />,
  );
}

describe("ChangesPanel filesOnly (folder projects)", () => {
  it("shows the file tree and never asks git for status", () => {
    setup(true);
    expect(screen.getByText("todo.md")).toBeDefined();
    expect(screen.getByRole("button", { name: "files" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "changes" })).toBeNull();
    expect(screen.queryByRole("button", { name: "commits" })).toBeNull();
    expect(q.useWorktreeGitStatus).toHaveBeenCalledWith("ws1", false);
  });

  it("keeps all three tabs, the memory button and the live status query for a git project", () => {
    setup(false);
    expect(screen.getByRole("button", { name: "changes" })).toBeDefined();
    expect(screen.getByRole("button", { name: "commits" })).toBeDefined();
    expect(screen.getByRole("button", { name: "files" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Project memory" })).toBeDefined();
    expect(q.useWorktreeGitStatus).toHaveBeenCalledWith("ws1", true);
  });

  it("shows the empty-commits card naming the base branch", () => {
    setup(false);
    fireEvent.click(screen.getByRole("button", { name: "commits" }));
    expect(screen.getByText("No commits yet")).toBeDefined();
    expect(screen.getByText("main")).toBeDefined();
  });

  it("gives folder projects a memory button that fetches only once opened", () => {
    setup(true);
    expect(screen.getByText("todo.md")).toBeDefined();
    expect(q.useProjectMemory).toHaveBeenLastCalledWith("ws1", false);

    fireEvent.click(screen.getByRole("button", { name: "Project memory" }));
    expect(q.useProjectMemory).toHaveBeenLastCalledWith("ws1", true);
    expect(screen.getByText("/c/projects/-r/memory")).toBeDefined();
    expect(screen.queryByText("todo.md")).toBeNull();
  });
});
