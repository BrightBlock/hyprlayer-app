// @vitest-environment jsdom
import { act, cleanup, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runCommand, useHasCommand } from "@/lib/keybindings/registry";
import { useUiStore } from "@/lib/uiStore";
import { LeadDockSplit } from "./LeadSplit";

vi.mock("./LeadPanel", () => ({ LeadPanel: () => <div aria-label="Lead" /> }));

beforeEach(() => {
  localStorage.clear();
  useUiStore.setState({ leadPanelHidden: false, leadDockWidth: 300 });
});
afterEach(cleanup);

describe("Lead panel collapse in the dock", () => {
  it("offers the toggle command only while the dock is open, and hides the panel and splitter", () => {
    const offered = renderHook(() => useHasCommand("view.toggleLeadPanel"));
    expect(offered.result.current).toBe(false);

    const dock = render(
      <LeadDockSplit projectPath="/p" views={[]}>
        <div>chat</div>
      </LeadDockSplit>,
    );
    offered.rerender();
    expect(offered.result.current).toBe(true);

    act(() => void runCommand("view.toggleLeadPanel"));
    expect(screen.queryByLabelText("Lead")).toBeNull();
    expect(screen.queryByRole("separator")).toBeNull();
    expect(JSON.parse(localStorage.getItem("hyprlayer:ui")!).state.leadPanelHidden).toBe(true);

    act(() => void runCommand("view.toggleLeadPanel"));
    expect(screen.getByLabelText("Lead")).toBeTruthy();

    dock.unmount();
    offered.rerender();
    expect(offered.result.current).toBe(false);
  });
});
