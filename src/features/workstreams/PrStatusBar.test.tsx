// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PrStatusBar } from "./PrStatusBar";
import type { PrStatus, WorkstreamView } from "@/lib/bindings";

// Mock the data hooks so the bar renders off a plain PrStatus — no react-query,
// no gh subprocess. `vi.hoisted` lets the (hoisted) vi.mock factory reach them.
const q = vi.hoisted(() => ({
  useWorkstreamPr: vi.fn(),
  useMergePr: vi.fn(),
  useBootstrap: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
vi.mock("@/features/session/sessionStore", () => ({ addressPrFeedback: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

afterEach(cleanup);

const VIEW = { item: { id: "ws1" } } as unknown as WorkstreamView;

function makePr(overrides: Partial<PrStatus> = {}): PrStatus {
  return {
    number: 42,
    url: "https://github.com/o/r/pull/42",
    title: "My PR",
    state: "open",
    mergeable: true,
    mergeableState: "mergeable",
    checks: "passing",
    checksTotal: 1,
    checksPassed: 1,
    checksFailed: 0,
    checksList: [],
    reviewDecision: null,
    ...overrides,
  };
}

function renderBar(pr: PrStatus, automerge = false) {
  q.useWorkstreamPr.mockReturnValue({ data: pr });
  q.useMergePr.mockReturnValue({ mutate: vi.fn(), isPending: false });
  q.useBootstrap.mockReturnValue({ data: { settings: { git: { automerge } } } });
  render(<PrStatusBar view={VIEW} onArchive={vi.fn()} />);
}

const mergeBtn = () => screen.getByRole("button", { name: "Merge" }) as HTMLButtonElement;

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
      "A reviewer requested changes — address them before merging",
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
    // Stays a plain, disabled Merge — no Auto-merge affordance.
    expect(screen.queryByRole("button", { name: "Auto-merge" })).toBeNull();
    expect(mergeBtn().disabled).toBe(true);
  });
});

describe("PrStatusBar merge menu", () => {
  it("floats fixed and right-aligned to the Merge button", () => {
    renderBar(makePr());
    const btn = mergeBtn();
    // jsdom reports a zero rect; stand in for a button inside the right sidebar.
    btn.getBoundingClientRect = () => ({ bottom: 120, right: 900 }) as DOMRect;
    Object.defineProperty(window, "innerWidth", { value: 1000, configurable: true });

    fireEvent.click(btn);
    const menu = screen.getByText("Squash and merge").closest("div");

    // Must be `fixed`, not `absolute`: the diff sidebar is `overflow-hidden`
    // (for its width animation) and clips the menu at its left edge.
    expect(menu?.className).toContain("fixed");
    expect(menu?.className).not.toContain("absolute");
    expect(menu?.style.top).toBe("124px");
    expect(menu?.style.right).toBe("100px");
  });
});
