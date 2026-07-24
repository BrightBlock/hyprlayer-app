// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChangesPanel } from "./ChangesPanel";

// Plain values instead of react-query/IPC, so the panel runs its real logic.
const q = vi.hoisted(() => ({
  useWorktreeGitStatus: vi.fn(),
  useWorktreeDir: vi.fn(),
  useStageFiles: vi.fn(),
  useUnstageFiles: vi.fn(),
  useDiscardFiles: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
vi.mock("@/lib/queryClient", () => ({ queryClient: { invalidateQueries: vi.fn() } }));
vi.mock("@/features/session/sessionStore", () => ({ useWorkstreamDocs: () => [] }));

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

  render(
    <ChangesPanel
      workstreamId="ws1"
      onOpenDoc={vi.fn()}
      onOpenDiff={vi.fn()}
      onOpenFile={vi.fn()}
      filesOnly={filesOnly}
    />,
  );
}

describe("ChangesPanel filesOnly (folder projects)", () => {
  it("shows the file tree and never asks git for status", () => {
    setup(true);
    // The tree is the whole panel — files are browsable/openable.
    expect(screen.getByText("todo.md")).toBeDefined();
    expect(screen.getByText("Files")).toBeDefined();
    // No Changes tab to switch to: a folder project has no staged/unstaged.
    expect(screen.queryByRole("button", { name: "changes" })).toBeNull();
    // The git-status query must be DISABLED, not merely unrendered — a folder
    // has no repo to answer it.
    expect(q.useWorktreeGitStatus).toHaveBeenCalledWith("ws1", false);
  });

  it("keeps both tabs and the live status query for a git project", () => {
    setup(false);
    expect(screen.getByRole("button", { name: "changes" })).toBeDefined();
    expect(screen.getByRole("button", { name: "files" })).toBeDefined();
    expect(q.useWorktreeGitStatus).toHaveBeenCalledWith("ws1", true);
  });
});
