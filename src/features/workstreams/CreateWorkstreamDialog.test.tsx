// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CreateWorkstreamDialog } from "./CreateWorkstreamDialog";
import type { ArchivedWorkstreamView, WorkstreamBases } from "@/lib/bindings";

// Mock the data hooks so the dialog runs its real state logic against a plain
// bases object, no react-query, no git subprocess. `vi.hoisted` lets the
// (hoisted) vi.mock factory reach them.
const q = vi.hoisted(() => ({
  useWorkstreamBases: vi.fn(),
  useCreateWorkstream: vi.fn(),
  useFetchProjectRemote: vi.fn(),
  useArchivedWorkstreams: vi.fn(),
  useDiscardProjectChanges: vi.fn(),
  useRestoreWorkstream: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);

afterEach(cleanup);

type BasesResult = {
  data: WorkstreamBases;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => void;
};

function basesResult(data: Partial<WorkstreamBases>, refetch = vi.fn()): BasesResult {
  return {
    data: {
      branches: [],
      localBranches: [],
      remoteBranches: [],
      defaultBase: "",
      currentBranch: "",
      ...data,
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch,
  };
}

function mockBases(data: Partial<WorkstreamBases>, refetch = vi.fn()) {
  q.useWorkstreamBases.mockReturnValue(basesResult(data, refetch));
}

// Stable no-op mutations. `create.mutate` is re-stubbed per test that asserts on
// what the dialog submits.
const createMutate = vi.fn();
function mockCreateOk() {
  q.useCreateWorkstream.mockReturnValue({
    isPending: false,
    isError: false,
    error: null,
    reset: vi.fn(),
    mutate: createMutate,
  });
}
beforeEach(mockCreateOk);
mockCreateOk();
const fetchRemoteMutate = vi.fn();
q.useFetchProjectRemote.mockReturnValue({ mutate: fetchRemoteMutate, isPending: false });
const discardMutate = vi.fn();
const restoreMutate = vi.fn();
q.useArchivedWorkstreams.mockReturnValue({ data: [] });
q.useDiscardProjectChanges.mockReturnValue({
  mutate: discardMutate,
  reset: vi.fn(),
  isPending: false,
  isError: false,
  error: null,
});
q.useRestoreWorkstream.mockReturnValue({
  mutate: restoreMutate,
  reset: vi.fn(),
  isPending: false,
  isError: false,
  error: null,
});

/** Put the create mutation into its `checkout_dirty` failure state. */
function mockCheckoutDirty(count = 2) {
  q.useCreateWorkstream.mockReturnValue({
    isPending: false,
    isError: true,
    error: {
      code: "checkout_dirty",
      message: `the project checkout has ${count} uncommitted change(s)`,
      remedy: null,
    },
    reset: vi.fn(),
    mutate: createMutate,
  });
}

function renderDialog(open: boolean) {
  return render(
    <CreateWorkstreamDialog
      projectPath="/repo"
      open={open}
      onClose={vi.fn()}
      onCreated={vi.fn()}
    />,
  );
}

// The "Branched from" base picker. It's a DropdownSelect, so the trigger is a
// button whose label IS the current choice, and the options only exist in the
// DOM while the menu is open.
const baseTrigger = () => screen.getByRole("button", { name: "Branched from" });
const baseValue = () => baseTrigger().textContent;
/** Open the menu and click one option, the DropdownSelect equivalent of
 *  `fireEvent.change(select, …)`. */
function pickBase(label: string) {
  fireEvent.click(baseTrigger());
  fireEvent.click(screen.getByRole("option", { name: label }));
}

describe("CreateWorkstreamDialog remote branches", () => {
  it("offers a remote-only branch and submits its full ref", () => {
    createMutate.mockClear();
    mockBases({
      branches: ["main", "origin/main", "origin/linux-support"],
      localBranches: ["main"],
      remoteBranches: ["origin/linux-support"],
      defaultBase: "main",
      currentBranch: "main",
    });
    renderDialog(true);

    // The remote-only branch is pickable and flagged as such.
    const row = screen.getByRole("button", { name: /origin\/linux-support/ });
    expect(row.textContent).toContain("remote");
    fireEvent.click(row);

    // The hint names the local branch that will be created to track it.
    expect(
      screen.getByText(/Creates local branch linux-support tracking origin\/linux-support/),
    ).toBeDefined();

    fireEvent.change(screen.getByPlaceholderText("What are you working on?"), {
      target: { value: "Linux" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    // The backend resolves the ref to a local tracking branch, so the dialog
    // hands it the ref verbatim.
    expect(createMutate).toHaveBeenCalledWith(
      { title: "Linux", mode: "existing", base: null, branch: "origin/linux-support" },
      expect.anything(),
    );
  });

  it("fetches the remote on the open edge, not while closed", () => {
    fetchRemoteMutate.mockClear();
    mockBases({ branches: ["main"], localBranches: ["main"], defaultBase: "main" });

    const { rerender } = renderDialog(false);
    expect(fetchRemoteMutate).not.toHaveBeenCalled();

    rerender(
      <CreateWorkstreamDialog projectPath="/repo" open onClose={vi.fn()} onCreated={vi.fn()} />,
    );
    expect(fetchRemoteMutate).toHaveBeenCalledTimes(1);
  });

  it("skips the fetch for a folder project, which has no branches", () => {
    fetchRemoteMutate.mockClear();
    mockBases({});
    render(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        folder
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(fetchRemoteMutate).not.toHaveBeenCalled();
  });
});

/** An archived in-place row on `branch`, as `list_archived_workstreams` returns it. */
function archivedRow(id: string, title: string, branch: string): ArchivedWorkstreamView {
  return {
    item: {
      id,
      projectPath: "/repo",
      title,
      slug: `${id}-abcd`,
      branch,
      worktreePath: "/repo",
      baseBranch: "origin/main",
      worktreeMode: "inplace",
      stage: "general",
      status: "archived",
      prNumber: null,
      createdAt: 1,
      updatedAt: 2,
      archivedAt: 3,
    },
    archivePath: "/repo/archived-contexts/x",
    archiveExists: true,
  };
}

describe("CreateWorkstreamDialog checkout-dirty recovery", () => {
  it("offers to unarchive the workstream whose branch the checkout is still on", () => {
    restoreMutate.mockClear();
    mockCheckoutDirty();
    mockBases({ localBranches: ["feature/x"], currentBranch: "feature/x" });
    q.useArchivedWorkstreams.mockReturnValue({
      data: [
        archivedRow("newest", "Agent work", "feature/x"),
        // Same branch but older, and a same-age row on another branch: neither
        // is the one that left the changes.
        archivedRow("older", "Earlier pass", "feature/x"),
        archivedRow("other", "Unrelated", "main"),
      ],
    });
    renderDialog(true);

    // The row names the owner and its archive age, with one Unarchive action.
    expect(screen.getByText("Agent work")).toBeDefined();
    // The newest matching row wins (the backend lists archived newest-first).
    fireEvent.click(screen.getByRole("button", { name: "Unarchive" }));
    expect(restoreMutate).toHaveBeenCalledWith("newest", expect.anything());
  });

  it("offers discard alone when no archived row claims the branch", () => {
    mockCheckoutDirty();
    mockBases({ localBranches: ["feature/x"], currentBranch: "feature/x" });
    q.useArchivedWorkstreams.mockReturnValue({ data: [archivedRow("other", "Unrelated", "main")] });
    renderDialog(true);

    expect(screen.queryByRole("button", { name: "Unarchive" })).toBeNull();
    expect(screen.getByRole("button", { name: "Discard 2 changes" })).toBeDefined();
  });

  it("names the change count and confirms in a second dialog before discarding", () => {
    discardMutate.mockClear();
    mockCheckoutDirty(1);
    mockBases({ localBranches: ["feature/x"], currentBranch: "feature/x" });
    q.useArchivedWorkstreams.mockReturnValue({ data: [] });
    renderDialog(true);

    fireEvent.click(screen.getByRole("button", { name: "Discard 1 change" }));
    expect(discardMutate).not.toHaveBeenCalled();
    // The confirm states what is destroyed, and what survives.
    expect(screen.getByText("Discard changes?")).toBeDefined();
    expect(screen.getByText(/untracked files are left alone/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(discardMutate).toHaveBeenCalledTimes(1);
  });

  it("explains the block without an error box", () => {
    mockCheckoutDirty(3);
    mockBases({ localBranches: ["feature/x"], currentBranch: "feature/x" });
    q.useArchivedWorkstreams.mockReturnValue({ data: [] });
    renderDialog(true);

    expect(screen.getByText("Your checkout has uncommitted work")).toBeDefined();
    expect(screen.getByText("3 changes on feature/x")).toBeDefined();
    // The raw backend string is replaced by the notice, not shown alongside it.
    expect(screen.queryByText(/uncommitted change\(s\)/)).toBeNull();
  });

  it("leaves an unrelated create failure as a plain error", () => {
    q.useCreateWorkstream.mockReturnValue({
      isPending: false,
      isError: true,
      error: { code: "inplace_exists", message: "already has one", remedy: null },
      reset: vi.fn(),
      mutate: createMutate,
    });
    mockBases({ localBranches: ["main"], currentBranch: "main" });
    renderDialog(true);

    expect(screen.getByText("already has one")).toBeDefined();
    expect(screen.queryByRole("button", { name: /^Discard/ })).toBeNull();
  });
});

describe("CreateWorkstreamDialog base detection", () => {
  it("re-scans local branches on the open edge, not while closed", () => {
    const refetch = vi.fn();
    mockBases({ branches: ["dev"], localBranches: ["dev"], defaultBase: "dev" }, refetch);

    const { rerender } = renderDialog(false);
    expect(refetch).not.toHaveBeenCalled();

    rerender(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("adopts a main branch that only appears on the open refetch", () => {
    // First paint: repo has no main, the base falls back to the first branch.
    mockBases({
      branches: ["feature-x"],
      localBranches: ["feature-x"],
      defaultBase: "",
      currentBranch: "feature-x",
    });
    const { rerender } = renderDialog(true);
    expect(baseValue()).toBe("feature-x");

    // The open refetch lands: `main` now exists on disk and is the detected
    // default. The picker must follow it, the whole point of the fix.
    mockBases({
      branches: ["main", "feature-x"],
      localBranches: ["main", "feature-x"],
      defaultBase: "main",
      currentBranch: "feature-x",
    });
    rerender(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(baseValue()).toBe("main");
    // And `main` is offered as an option.
    fireEvent.click(baseTrigger());
    expect(screen.getByRole("option", { name: "main" })).toBeDefined();
  });

  it("keeps a manually chosen base when fresh branches arrive", () => {
    mockBases({
      branches: ["dev", "feature-x"],
      localBranches: ["dev", "feature-x"],
      defaultBase: "dev",
      currentBranch: "feature-x",
    });
    const { rerender } = renderDialog(true);
    expect(baseValue()).toBe("dev");

    // User overrides the default.
    pickBase("feature-x");
    expect(baseValue()).toBe("feature-x");

    // A later refetch surfaces `main` as the new default, the manual pick wins.
    mockBases({
      branches: ["main", "dev", "feature-x"],
      localBranches: ["main", "dev", "feature-x"],
      defaultBase: "main",
      currentBranch: "feature-x",
    });
    rerender(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(baseValue()).toBe("feature-x");
  });
});
