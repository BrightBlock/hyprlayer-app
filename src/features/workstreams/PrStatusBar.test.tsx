// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PrStatusBar } from "./PrStatusBar";
import type { PrLookup, PrStatus, WorkstreamView } from "@/lib/bindings";

const q = vi.hoisted(() => ({
  useWorkstreamPr: vi.fn(),
  useMergePr: vi.fn(),
  useBootstrap: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
const mergeWorkstreamPr = vi.hoisted(() => vi.fn());
vi.mock("@/lib/bindings", async (orig) => {
  const actual = await orig<typeof import("@/lib/bindings")>();
  return { ...actual, commands: { ...actual.commands, mergeWorkstreamPr } };
});
vi.mock("@/features/session/sessionStore", () => ({ addressPrFeedback: vi.fn() }));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

afterEach(cleanup);

const VIEW = { item: { id: "ws1" } } as unknown as WorkstreamView;

function makePr(overrides: Partial<PrStatus> = {}): PrStatus {
  return {
    number: 42,
    url: "https://github.com/o/r/pull/42",
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

function renderBar(pr: PrStatus, automerge = false) {
  renderLookup({ kind: "found", pr }, automerge);
}

function renderLookup(lookup: PrLookup, automerge = false) {
  q.useWorkstreamPr.mockReturnValue({ data: lookup });
  q.useMergePr.mockReturnValue({ mutate: vi.fn(), isPending: false });
  q.useBootstrap.mockReturnValue({ data: { settings: { git: { automerge } } } });
  render(<PrStatusBar view={VIEW} onArchive={vi.fn()} />);
}

const mergeBtn = () => screen.getByRole("button", { name: "Merge" }) as HTMLButtonElement;

describe("PrStatusBar PR link", () => {
  it("opens the PR on GitHub when the PR pill is clicked", () => {
    renderBar(makePr());
    fireEvent.click(screen.getByText("PR #42"));
    expect(opener.openUrl).toHaveBeenCalledWith("https://github.com/o/r/pull/42");
  });
});

describe("PrStatusBar merge gate", () => {
  it("enables Merge when checks pass and no review is required", () => {
    renderBar(makePr({ reviewDecision: null }));
    expect(mergeBtn().disabled).toBe(false);
  });

  it("blocks Merge while a required review is outstanding", () => {
    renderBar(makePr({ reviewDecision: "reviewRequired" }));
    const btn = mergeBtn();
    expect(btn.disabled).toBe(true);
    expect(btn.getAttribute("title")).toBe(
      "Waiting for a required review before this can merge",
    );
  });

  it("re-enables Merge once the PR is approved", () => {
    renderBar(makePr({ reviewDecision: "approved" }));
    expect(mergeBtn().disabled).toBe(false);
  });

  it("blocks Merge and offers Address comments when changes are requested", () => {
    renderBar(makePr({ reviewDecision: "changesRequested" }));
    const btn = mergeBtn();
    expect(btn.disabled).toBe(true);
    expect(btn.getAttribute("title")).toBe(
      "A reviewer requested changes. Address them before merging",
    );
    expect(screen.getByText("Address comments")).toBeTruthy();
  });

  it("queues auto-merge for a pending review when automerge is on", () => {
    renderBar(makePr({ reviewDecision: "reviewRequired" }), true);
    const btn = screen.getByRole("button", { name: "Auto-merge" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
  });

  it("does not queue auto-merge for changes-requested (author's turn)", () => {
    renderBar(makePr({ reviewDecision: "changesRequested" }), true);
    expect(screen.queryByRole("button", { name: "Auto-merge" })).toBeNull();
    expect(mergeBtn().disabled).toBe(true);
  });
});

describe("PrStatusBar checks", () => {
  it("shows no Fix CI and enables Merge for a green rollup like facebook/react#38314", () => {
    renderBar(
      makePr({
        checks: { state: "passing", total: 244, passed: 243, failed: 0, pending: 0, skipped: 1, cancelled: 0, list: [] },
      }),
    );
    expect(screen.queryByText("Fix CI")).toBeNull();
    expect(mergeBtn().disabled).toBe(false);
  });

  it("offers Fix CI and no Auto-merge for a failed check while another runs", () => {
    renderBar(
      makePr({
        checks: { state: "failing", total: 2, passed: 0, failed: 1, pending: 1, skipped: 0, cancelled: 0, list: [] },
      }),
      true,
    );
    expect(screen.getByText("Fix CI")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Auto-merge" })).toBeNull();
    expect(mergeBtn().disabled).toBe(true);
  });
});

describe("PrStatusBar merge state", () => {
  it("disables Merge on a draft like OISF/suricata#15352 and says why", () => {
    renderBar(makePr({ isDraft: true, mergeState: "blocked", reviewDecision: "reviewRequired" }), true);
    const btn = screen.getByRole("button", { name: /merge/i }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.getAttribute("title")).toMatch(/^Draft/);
  });

  it("gives blocked, behind and conflicting PRs their own reason", () => {
    const titleFor = (pr: PrStatus) => {
      renderBar(pr);
      const title = mergeBtn().getAttribute("title");
      expect(mergeBtn().disabled).toBe(true);
      cleanup();
      return title;
    };
    expect(titleFor(makePr({ mergeState: "dirty" }))).toBe(
      "Merge conflicts with main. Resolve them before merging",
    );
    expect(titleFor(makePr({ mergeState: "behind", baseRefName: "release" }))).toBe(
      "This branch is behind release. Update it before merging",
    );
    expect(titleFor(makePr({ mergeState: "blocked" }))).toBe(
      "GitHub reports the merge is blocked by branch protection",
    );
    expect(titleFor(makePr({ mergeState: "unknown" }))).toBe(
      "GitHub is still computing whether this can merge",
    );
  });
});

describe("PrStatusBar quota icon", () => {
  const quotaIcon = () => screen.queryByRole("img", { name: /^GitHub quota low/ });

  it("marks a PR held by a low quota with an icon that says when it refreshes", () => {
    const resumesAt = Date.now() + 60_000;
    renderLookup({ kind: "failed", cause: { kind: "rateLimited", resumesAt }, last: makePr() });
    const at = new Date(resumesAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    expect(quotaIcon()?.getAttribute("title")).toBe(`GitHub quota low, refreshing at ${at}`);
    expect(quotaIcon()?.getAttribute("aria-label")).toBe(`GitHub quota low, refreshing at ${at}`);
    expect(screen.queryByText(/Stale:/)).toBeNull();
  });

  it("says only that the quota is low when GitHub gave no reset time", () => {
    renderLookup({ kind: "failed", cause: { kind: "rateLimited", resumesAt: null }, last: makePr() });
    expect(quotaIcon()?.getAttribute("title")).toBe("GitHub quota low");
  });

  it("uses text, not the icon, for other failures and none for a fresh lookup", () => {
    renderLookup({ kind: "failed", cause: { kind: "signedOut" }, last: makePr() });
    expect(quotaIcon()).toBeNull();
    expect(screen.getByText("Stale: gh is not signed in")).toBeTruthy();
    cleanup();
    renderBar(makePr());
    expect(quotaIcon()).toBeNull();
    expect(screen.queryByText(/Stale:/)).toBeNull();
  });
});

describe("PrStatusBar merge menu", () => {
  it("floats fixed and right-aligned to the Merge button", () => {
    renderBar(makePr());
    const btn = mergeBtn();
    btn.getBoundingClientRect = () => ({ bottom: 120, right: 900 }) as DOMRect;
    Object.defineProperty(window, "innerWidth", { value: 1000, configurable: true });

    fireEvent.click(btn);
    const menu = screen.getByText("Squash and merge").closest("div");

    expect(menu?.className).toContain("fixed");
    expect(menu?.className).not.toContain("absolute");
    expect(menu?.style.top).toBe("124px");
    expect(menu?.style.left).toBe("900px");
    expect(menu?.style.right).toBe("");
  });
});

describe("PrStatusBar merge failure", () => {
  it("tells the user why a merge failed", async () => {
    const actual = await vi.importActual<typeof import("@/lib/queries")>("@/lib/queries");
    mergeWorkstreamPr.mockResolvedValue({
      status: "error",
      error: {
        code: "gh_failed",
        message: "Pull request #42 is not mergeable: base branch policy prohibits the merge",
        remedy: null,
      },
    });
    q.useWorkstreamPr.mockReturnValue({ data: { kind: "found", pr: makePr() } });
    q.useMergePr.mockImplementation(actual.useMergePr);
    q.useBootstrap.mockReturnValue({ data: { settings: { git: { automerge: false } } } });
    render(
      <QueryClientProvider client={new QueryClient()}>
        <PrStatusBar view={VIEW} onArchive={vi.fn()} />
      </QueryClientProvider>,
    );
    fireEvent.click(mergeBtn());
    fireEvent.click(screen.getByText("Squash and merge"));
    expect(await screen.findByText(/not mergeable/)).toBeTruthy();
    expect(mergeWorkstreamPr).toHaveBeenCalledTimes(1);
  });
});
