// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GitPanel } from "./GitPanel";
import type { PrLookup, PrStatus, WorkstreamView } from "@/lib/bindings";

const q = vi.hoisted(() => {
  const mutation = () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), reset: vi.fn(), isPending: false, isError: false });
  return {
    useWorkstreamPr: vi.fn(),
    useMergePr: vi.fn(mutation),
    useBootstrap: vi.fn(() => ({ data: { settings: { git: { automerge: false } } } })),
    useFetchWorkstreamRemote: vi.fn(mutation),
    useSetWorkstreamBase: vi.fn(mutation),
    useUpdateFromBase: vi.fn(mutation),
    useWorkstreamBases: vi.fn(() => ({ data: undefined, isLoading: false, isFetching: false, refetch: vi.fn() })),
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
  git: { exists: true, branch: "feat", ahead: 1, behind: 0, aheadOfBase: 1 },
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

function renderPanel(lookup: PrLookup) {
  q.useWorkstreamPr.mockReturnValue({ data: lookup });
  render(<GitPanel view={VIEW} />);
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
