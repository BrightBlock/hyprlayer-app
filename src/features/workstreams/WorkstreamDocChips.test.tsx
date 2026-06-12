// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { WorkstreamDocChips, chipLabel } from "./WorkstreamDocChips";

// No vitest globals -> RTL can't register its own auto-cleanup; without this,
// a previous test's mounted copy steals the next test's queries and clicks.
afterEach(cleanup);

const RESEARCH = {
  path: "/t/repos/p/shared/research/2026-06-12-auth-research.md",
  role: "research",
};
const PLAN = {
  path: "/t/repos/p/shared/plans/2026-06-12-auth-plan.md",
  role: "plan",
};

function renderChips(docs = [RESEARCH, PLAN]) {
  const onOpenDoc = vi.fn();
  const onDetach = vi.fn();
  const onBrowse = vi.fn();
  render(
    <WorkstreamDocChips docs={docs} onOpenDoc={onOpenDoc} onDetach={onDetach} onBrowse={onBrowse} />,
  );
  return { onOpenDoc, onDetach, onBrowse };
}

describe("chipLabel", () => {
  it("strips directories, date prefix, and extension", () => {
    expect(chipLabel("/x/2026-06-12-auth-research.md")).toBe("auth-research");
    expect(chipLabel("/x/AUTH-12-notes.md")).toBe("AUTH-12-notes");
    expect(chipLabel("/x/2026-06-12.md")).toBe("2026-06-12");
  });
});

describe("WorkstreamDocChips", () => {
  it("renders one chip per attached doc of THIS tab", () => {
    renderChips();
    expect(screen.getByText("auth-research")).toBeTruthy();
    expect(screen.getByText("auth-plan")).toBeTruthy();
    expect(screen.getByText("Attach")).toBeTruthy();
  });

  it("opens a doc when its chip is clicked", () => {
    const { onOpenDoc } = renderChips();
    fireEvent.click(screen.getByText("auth-research"));
    expect(onOpenDoc).toHaveBeenCalledWith(RESEARCH.path);
  });

  it("detaches via the chip's close button", () => {
    const { onDetach } = renderChips();
    fireEvent.click(screen.getByLabelText("Detach auth-research"));
    expect(onDetach).toHaveBeenCalledWith(RESEARCH.path);
  });

  it("offers 'Attach context' and browse-jump when nothing is attached", () => {
    const { onBrowse } = renderChips([]);
    fireEvent.click(screen.getByText("Attach context"));
    expect(onBrowse).toHaveBeenCalled();
  });
});
