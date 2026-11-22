// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GitPanel } from "./GitPanel";
import { WorkstreamGitChip } from "./WorkstreamGitChip";
import type { PrLookup, PrStatus, WorkstreamView, WorktreeStatus } from "@/lib/bindings";

const q = vi.hoisted(() => {
  const mutation = () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), reset: vi.fn(), isPending: false, isError: false });
  return {
    useWorkstreamPr: vi.fn(),
    useMergePr: vi.fn(mutation),
    useBootstrap: vi.fn(() => ({ data: { settings: { git: { automerge: false } } } })),
    useFetchWorkstreamRemote: vi.fn(mutation),
    useSetWorkstreamBase: vi.fn(mutation),
    useUpdateFromBase: vi.fn(mutation),
    usePullWorkstreamBranch: vi.fn(mutation),
    useWorkstreamBases: vi.fn(() => ({
      data: undefined as { branches: string[] } | undefined,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    })),
    useWorkstreamSessions: vi.fn(() => ({ data: [] })),
    useWorktreeGitStatus: vi.fn(() => ({ data: { staged: [], unstaged: [] } })),
  };
});
vi.mock("@/lib/queries", () => q);
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@/lib/appNav", () => ({ closeReview: vi.fn(), currentAppLocation: () => null }));
vi.mock("@/features/session/transcript", () => ({ isWorking: () => false }));
vi.mock("@/features/session/sessionStore", () => ({
  addressPrFeedback: vi.fn(),
  commitWorkstream: vi.fn(),
  shipWorkstream: vi.fn(),
  useSessionStore: (select: (s: { sessions: object }) => unknown) => select({ sessions: {} }),
  useWorkstreamShipBusy: () => false,
}));
vi.mock("./AttachPrDialog", () => ({ AttachPrDialog: () => null }));
vi.mock("./ArchiveWorkstreamDialog", () => ({ ArchiveWorkstreamDialog: () => null }));

afterEach(cleanup);

const VIEW = {
  item: {
    id: "ws1",
    projectPath: "/p",
    branch: "feat",
    baseBranch: "origin/main",
    worktreeMode: "isolated",
  },
  git: {
    exists: true,
    branch: "feat",
    additions: 0,
    deletions: 0,
    dirty: 0,
    base: { ahead: 1, behind: 0 },
    upstream: null,
    unpushed: 1,
  },
} as unknown as WorkstreamView;

function pr(overrides: Partial<PrStatus> = {}): PrStatus {
  return {
    number: 7,
    url: "https://github.com/o/r/pull/7",
    title: "My PR",
    state: "open",
    isDraft: false,
    baseRefName: "main",
    headRefOid: "abc",
    mergeState: "clean",
    checks: { state: "passing", total: 1, passed: 1, failed: 0, pending: 0, skipped: 0, cancelled: 0, list: [] },
    reviewDecision: null,
    reviews: [],
    ...overrides,
  };
}

let setBase: ReturnType<typeof vi.fn>;

beforeEach(() => {
  setBase = vi.fn();
  q.useSetWorkstreamBase.mockReturnValue({
    mutate: setBase,
    mutateAsync: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    isError: false,
  });
});

function withGit(git: Partial<WorktreeStatus>): WorkstreamView {
  return { ...VIEW, git: { ...VIEW.git, ...git } };
}

function renderPanel(lookup: PrLookup, view: WorkstreamView = VIEW) {
  q.useWorkstreamPr.mockReturnValue({ data: lookup });
  render(<GitPanel view={view} />);
}

const ship = () => screen.queryByRole("button", { name: "Ship" }) as HTMLButtonElement | null;
const quotaIcon = () => screen.queryByRole("img", { name: /^GitHub quota low/ });

describe("GitPanel PR row", () => {
  it("does not claim there is no PR when the quota ran out, and shows it as an icon", () => {
    renderPanel({ kind: "failed", cause: { kind: "rateLimited", resumesAt: null }, last: null });
    expect(screen.queryByText("Attach an existing PR…")).toBeNull();
    expect(screen.queryByText(/PR status unknown/)).toBeNull();
    expect(quotaIcon()?.getAttribute("title")).toBe("GitHub quota low");
    expect(ship()?.disabled).toBe(true);
    expect(ship()?.getAttribute("title")).toBe("PR status unknown: GitHub quota low");
  });

  it("keeps the last PR, with the quota icon, while GitHub's quota is low", () => {
    renderPanel({ kind: "failed", cause: { kind: "rateLimited", resumesAt: Date.now() + 60_000 }, last: pr() });
    expect(screen.getByText("PR #7")).toBeTruthy();
    expect(quotaIcon()?.getAttribute("aria-label")).toMatch(/^GitHub quota low, refreshing at \d/);
    expect(quotaIcon()?.getAttribute("title")).toBe(quotaIcon()?.getAttribute("aria-label"));
    expect(screen.queryByText(/Stale:/)).toBeNull();
    expect(ship()).toBeNull();
  });

  it("keeps the last PR, marked stale in text, when gh failed", () => {
    renderPanel({ kind: "failed", cause: { kind: "other" }, last: pr() });
    expect(screen.getByText("PR #7")).toBeTruthy();
    expect(screen.getByText("Stale: gh failed")).toBeTruthy();
    expect(quotaIcon()).toBeNull();
    expect(ship()).toBeNull();
  });

  it("says why the PR is unknown in text when gh is signed out", () => {
    renderPanel({ kind: "failed", cause: { kind: "signedOut" }, last: null });
    expect(screen.getByText("PR status unknown: gh is not signed in")).toBeTruthy();
    expect(quotaIcon()).toBeNull();
    expect(ship()?.disabled).toBe(true);
  });

  it("shows that the branch's PR was closed, and offers Ship", () => {
    renderPanel({ kind: "found", pr: pr({ number: 42, state: "closed" }) });
    expect(screen.getByText("PR #42")).toBeTruthy();
    expect(screen.getByText("Closed")).toBeTruthy();
    expect(ship()?.disabled).toBe(false);
  });

  it("hides the PR row and keeps Ship for a repository with no GitHub remote", () => {
    renderPanel({ kind: "notGitHub" });
    expect(screen.queryByText("Attach an existing PR…")).toBeNull();
    expect(screen.queryByText(/PR status unknown/)).toBeNull();
    expect(ship()?.disabled).toBe(false);
  });

  it("offers Attach and Ship for a branch with no PR of its own", () => {
    renderPanel({ kind: "noPr" });
    expect(screen.getByText("Attach an existing PR…")).toBeTruthy();
    expect(ship()?.disabled).toBe(false);
    expect(screen.queryByText("Archive")).toBeNull();
  });
});

describe("GitPanel branch row", () => {
  const renderOn = (branch: string | null) => {
    q.useWorkstreamPr.mockReturnValue({ data: { kind: "noPr" } });
    render(<GitPanel view={{ ...VIEW, git: { ...VIEW.git, branch } }} />);
  };

  it("shows the branch HEAD is on and names the workstream's branch in the title", () => {
    renderOn("feature/csp-1");
    const row = screen.getByTitle("feature/csp-1 (workstream branch feat) → main");
    expect(row.textContent).toBe("feature/csp-1");
  });

  it("shows only the branch when HEAD is on the workstream's branch", () => {
    renderOn("feat");
    expect(screen.getByTitle("feat → main").textContent).toBe("feat");
  });

  it("shows a detached HEAD as such", () => {
    renderOn(null);
    expect(screen.getByTitle(/^feat , HEAD is detached/).textContent).toBe("detached HEAD");
  });

  it("offers the base, never the workstream's own branch, as a target while HEAD is on the base", () => {
    q.useWorkstreamBases.mockReturnValue({
      data: { branches: ["origin/main", "origin/feat"] },
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    renderOn("main");
    fireEvent.click(screen.getByLabelText("Change target branch"));
    expect(screen.queryByRole("button", { name: /^main/ })).not.toBeNull();
    expect(screen.queryByRole("button", { name: /^feat/ })).toBeNull();
    q.useWorkstreamBases.mockReset();
  });
});

describe("GitPanel base row", () => {
  it("shows a draft's real base and fixes the mismatch in one click", () => {
    renderPanel({ kind: "found", pr: pr({ isDraft: true, baseRefName: "release" }) });
    expect(screen.getByLabelText("Change target branch").textContent).toContain("release");
    expect(
      screen.getByRole("button", { name: /merge/i }).getAttribute("title"),
    ).toMatch(/^Draft/);
    fireEvent.click(screen.getByRole("button", { name: "Use origin/release" }));
    expect(setBase).toHaveBeenCalledWith("origin/release");
  });

  it("shows no mismatch when the PR targets the workstream's base", () => {
    renderPanel({ kind: "found", pr: pr({ baseRefName: "main" }) });
    expect(screen.queryByRole("button", { name: /^Use origin\// })).toBeNull();
  });
});

describe("GitPanel ahead/behind", () => {
  const updateFromBase = () => screen.queryByRole("button", { name: "Update from origin/main" });
  const pullButton = () => screen.queryByRole("button", { name: /^Pull origin\// });

  it("labels the header with the branch it measures against, never 'base'", () => {
    renderPanel({ kind: "noPr" }, withGit({ base: { ahead: 1, behind: 0 } }));
    expect(screen.getByTitle("1 commit(s) ahead of main")).toBeTruthy();
    expect(screen.getByTitle("0 commit(s) behind main")).toBeTruthy();
    expect(screen.queryByTitle(/ base$/)).toBeNull();
  });

  it("offers Pull, not Update from base, for a teammate's push to the branch", () => {
    const pull = vi.fn();
    q.usePullWorkstreamBranch.mockReturnValueOnce({
      mutate: pull,
      mutateAsync: vi.fn(),
      reset: vi.fn(),
      isPending: false,
      isError: false,
    });
    renderPanel(
      { kind: "noPr" },
      withGit({
        base: { ahead: 1, behind: 0 },
        upstream: { name: "origin/feat", ahead: 0, behind: 1 },
        unpushed: 0,
      }),
    );
    expect(screen.getByText("↓1 from origin/feat")).toBeTruthy();
    expect(updateFromBase()).toBeNull();
    fireEvent.click(pullButton()!);
    expect(pull).toHaveBeenCalled();
  });

  it("shows a diverged branch without a Pull that could only fail", () => {
    renderPanel(
      { kind: "noPr" },
      withGit({
        base: { ahead: 2, behind: 0 },
        upstream: { name: "origin/feat", ahead: 1, behind: 1 },
        unpushed: 1,
      }),
    );
    expect(pullButton()).toBeNull();
    expect(
      screen.getByTitle("Diverged from origin/feat; rebase or merge in a terminal").textContent,
    ).toBe("↑1 ↓1 vs origin/feat");
  });

  it("offers Update from base once origin/main moves past a pushed branch", () => {
    renderPanel(
      { kind: "noPr" },
      withGit({
        base: { ahead: 1, behind: 2 },
        upstream: { name: "origin/feat", ahead: 0, behind: 0 },
        unpushed: 0,
      }),
    );
    expect(updateFromBase()).toBeTruthy();
    expect(screen.getByTitle("2 commit(s) behind main").textContent).toBe("↓2");
    expect(pullButton()).toBeNull();
  });

  it("shows a fresh, unpushed branch ahead of main with no Pull", () => {
    renderPanel({ kind: "noPr" }, withGit({ base: { ahead: 1, behind: 0 }, upstream: null }));
    expect(screen.getByTitle("1 commit(s) ahead of main").textContent).toBe("↑1");
    expect(pullButton()).toBeNull();
    expect(screen.queryByText(/from origin\//)).toBeNull();
  });
});

describe("WorkstreamGitChip", () => {
  it("counts unpushed commits, not commits ahead of the base", () => {
    const { container, rerender } = render(
      <WorkstreamGitChip view={withGit({ unpushed: 2, base: { ahead: 2, behind: 0 } })} />,
    );
    expect(container.textContent).toBe("↑2");
    rerender(<WorkstreamGitChip view={withGit({ unpushed: 0, base: { ahead: 3, behind: 0 } })} />);
    expect(container.textContent).toBe("");
  });

  it("never reads as clean when the checkout's changes can't be read", () => {
    const reason = "this checkout uses a sparse index, which Hyprlayer can't read";
    const { container } = render(<WorkstreamGitChip view={withGit({ unpushed: 0, unreadable: reason })} />);
    expect(container.textContent).toBe("?");
    expect(screen.getByTitle(reason)).toBeDefined();
  });
});
