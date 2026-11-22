// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PrChecksPanel } from "./PrChecksPanel";
import type { PrChecks, PrLookup, PrStatus, WorkstreamView } from "@/lib/bindings";

const q = vi.hoisted(() => ({ useWorkstreamPr: vi.fn() }));
vi.mock("@/lib/queries", () => q);
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

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
    checks: { state: "none", total: 0, passed: 0, failed: 0, pending: 0, skipped: 0, cancelled: 0, list: [] },
    reviewDecision: null,
    reviews: [],
    ...overrides,
  };
}

function renderPanel(pr: PrStatus) {
  const lookup: PrLookup = { kind: "found", pr };
  q.useWorkstreamPr.mockReturnValue({ data: lookup });
  render(<PrChecksPanel view={VIEW} />);
}

describe("PrChecksPanel review surfacing", () => {
  it("surfaces a Review-required row while waiting on a reviewer", () => {
    renderPanel(makePr({ reviewDecision: "reviewRequired" }));
    expect(screen.getByText("Review required")).toBeTruthy();
    expect(screen.getByText("review")).toBeTruthy();
  });

  it("surfaces a Changes-requested row", () => {
    renderPanel(makePr({ reviewDecision: "changesRequested" }));
    expect(screen.getByText("Changes requested")).toBeTruthy();
  });

  it("drops the review row once the PR is approved", () => {
    renderPanel(makePr({ reviewDecision: "approved" }));
    expect(screen.queryByText("Review required")).toBeNull();
    expect(screen.queryByText("Changes requested")).toBeNull();
    expect(screen.getByText("none yet")).toBeTruthy();
  });

  it("keeps a merge conflict red-precedent over a pending review", () => {
    renderPanel(
      makePr({ mergeState: "dirty", reviewDecision: "reviewRequired" }),
    );
    expect(screen.getByText("Merge conflict")).toBeTruthy();
    expect(screen.getByText("Review required")).toBeTruthy();
    expect(screen.getByText("conflict")).toBeTruthy();
  });
});

describe("PrChecksPanel header count", () => {
  const checks = (c: Omit<PrChecks, "state" | "list">, state: PrChecks["state"]): PrChecks => ({
    ...c,
    state,
    list: [{ name: "build", bucket: "fail", url: null }],
  });

  it("counts failures and running checks instead of passed/total", () => {
    renderPanel(
      makePr({ checks: checks({ total: 20, passed: 11, failed: 1, pending: 0, skipped: 8, cancelled: 0 }, "failing") }),
    );
    expect(screen.getByText("1 failing")).toBeTruthy();
    cleanup();
    renderPanel(
      makePr({ checks: checks({ total: 3, passed: 1, failed: 1, pending: 1, skipped: 0, cancelled: 0 }, "failing") }),
    );
    expect(screen.getByText("1 failing · 1 running")).toBeTruthy();
  });

  it("shows passed/total once nothing fails or runs", () => {
    renderPanel(
      makePr({ checks: checks({ total: 20, passed: 12, failed: 0, pending: 0, skipped: 8, cancelled: 0 }, "passing") }),
    );
    expect(screen.getByText("12/20")).toBeTruthy();
  });
});
