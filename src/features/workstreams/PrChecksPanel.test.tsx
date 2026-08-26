// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PrChecksPanel } from "./PrChecksPanel";
import type { PrStatus, WorkstreamView } from "@/lib/bindings";

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
    mergeable: true,
    mergeableState: "mergeable",
    checks: "none",
    checksTotal: 0,
    checksPassed: 0,
    checksFailed: 0,
    checksList: [],
    reviewDecision: null,
    ...overrides,
  };
}

function renderPanel(pr: PrStatus) {
  q.useWorkstreamPr.mockReturnValue({ data: pr });
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
      makePr({ mergeable: false, mergeableState: "conflicting", reviewDecision: "reviewRequired" }),
    );
    expect(screen.getByText("Merge conflict")).toBeTruthy();
    expect(screen.getByText("Review required")).toBeTruthy();
    expect(screen.getByText("conflict")).toBeTruthy();
  });
});
