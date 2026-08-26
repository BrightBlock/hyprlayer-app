// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CreateWorkstreamDialog } from "./CreateWorkstreamDialog";
import type { ArchivedWorkstreamView, WorkstreamBases } from "@/lib/bindings";

const q = vi.hoisted(() => ({
  useWorkstreamBases: vi.fn(),
  useCreateWorkstream: vi.fn(),
  useFetchProjectRemote: vi.fn(),
  useArchivedWorkstreams: vi.fn(),
  useDiscardProjectChanges: vi.fn(),
  useRestoreWorkstream: vi.fn(),
  useBranchPrefixes: vi.fn(),
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

function mockPrefixes(
  prefixes = [
    { label: "Bug fix", prefix: "fix/", keywords: ["fix", "bug", "crash"] },
    { label: "Feature", prefix: "feat/", keywords: ["feature", "add"] },
  ],
) {
  q.useBranchPrefixes.mockReturnValue({
    data: { prefixes, defaultPrefix: "hyprlayer/", repo: [], source: "builtin" },
    isLoading: false,
    isError: false,
    error: null,
  });
}
mockPrefixes();

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

const baseTrigger = () => screen.getByRole("button", { name: "Branched from" });
const baseValue = () => baseTrigger().textContent;
function pickBase(label: string) {
  fireEvent.click(baseTrigger());
  fireEvent.click(screen.getByRole("option", { name: label }));
}

const typeTrigger = () => screen.getByRole("button", { name: "Type of work" });
const typeValue = () => typeTrigger().textContent;
function pickType(label: string) {
  fireEvent.click(typeTrigger());
  fireEvent.click(screen.getByRole("option", { name: label }));
}

const fillTitle = (value: string) =>
  fireEvent.change(screen.getByPlaceholderText("What are you working on?"), {
    target: { value },
  });

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

    const row = screen.getByRole("button", { name: /origin\/linux-support/ });
    expect(row.textContent).toContain("remote");
    fireEvent.click(row);

    expect(
      screen.getByText(/Creates local branch linux-support tracking origin\/linux-support/),
    ).toBeDefined();

    fireEvent.change(screen.getByPlaceholderText("What are you working on?"), {
      target: { value: "Linux" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(createMutate).toHaveBeenCalledWith(
      {
        title: "Linux",
        mode: "existing",
        base: null,
        branch: "origin/linux-support",
        branchPrefix: null,
      },
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
        archivedRow("older", "Earlier pass", "feature/x"),
        archivedRow("other", "Unrelated", "main"),
      ],
    });
    renderDialog(true);

    expect(screen.getByText("Agent work")).toBeDefined();
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
    mockBases({
      branches: ["feature-x"],
      localBranches: ["feature-x"],
      defaultBase: "",
      currentBranch: "feature-x",
    });
    const { rerender } = renderDialog(true);
    expect(baseValue()).toBe("feature-x");

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

    pickBase("feature-x");
    expect(baseValue()).toBe("feature-x");

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

describe("CreateWorkstreamDialog branch types", () => {
  const gitRepo = {
    branches: ["main"],
    localBranches: ["main"],
    defaultBase: "main",
    currentBranch: "main",
  };

  it("suggests a type from the title and submits its prefix", () => {
    createMutate.mockClear();
    mockBases(gitRepo);
    renderDialog(true);

    expect(typeValue()).toBe("General");

    fillTitle("Fix the login crash");
    expect(typeValue()).toBe("Bug fix");

    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(createMutate).toHaveBeenCalledWith(
      {
        title: "Fix the login crash",
        mode: "isolated",
        base: "main",
        branch: null,
        branchPrefix: "fix/",
      },
      expect.anything(),
    );
  });

  it("keeps an explicit None through further typing", () => {
    createMutate.mockClear();
    mockBases(gitRepo);
    renderDialog(true);

    fillTitle("Fix the login crash");
    expect(typeValue()).toBe("Bug fix");

    pickType("General");
    fillTitle("Fix the login crash on add");
    expect(typeValue()).toBe("General");

    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(createMutate).toHaveBeenCalledWith(
      {
        title: "Fix the login crash on add",
        mode: "isolated",
        base: "main",
        branch: null,
        branchPrefix: null,
      },
      expect.anything(),
    );
  });

  it("previews the branch the create will produce", () => {
    mockBases(gitRepo);
    renderDialog(true);

    fillTitle("Fix the login crash");
    expect(screen.getByText(/Preview:/).textContent).toBe(
      "Preview: fix/fix-the-login-crash-xxxx",
    );
  });

  it("falls back to the global prefix when no keyword hits", () => {
    createMutate.mockClear();
    mockBases(gitRepo);
    renderDialog(true);

    fillTitle("Dashboard tweaks");
    expect(typeValue()).toBe("General");
    expect(screen.getByText(/Preview:/).textContent).toBe(
      "Preview: hyprlayer/dashboard-tweaks-xxxx",
    );

    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(createMutate).toHaveBeenCalledWith(
      {
        title: "Dashboard tweaks",
        mode: "isolated",
        base: "main",
        branch: null,
        branchPrefix: null,
      },
      expect.anything(),
    );
  });

  it("hides the picker for a branch that already has a name", () => {
    mockBases({ ...gitRepo, branches: ["main", "feature-x"], localBranches: ["main", "feature-x"] });
    renderDialog(true);
    expect(typeTrigger()).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: /feature-x/ }));
    expect(screen.queryByRole("button", { name: "Type of work" })).toBeNull();

    cleanup();
    render(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        folder
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "Type of work" })).toBeNull();
  });
});
