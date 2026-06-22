// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PromptHistoryStack } from "./PromptHistoryStack";

afterEach(cleanup);

const SAMPLE = ["first prompt", "second prompt", "third prompt"];

describe("PromptHistoryStack", () => {
  it("renders nothing for an empty prompts array", () => {
    const { container } = render(
      <PromptHistoryStack prompts={[]} activeIndex={null} onRecall={vi.fn()} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders one button per prompt", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={null} onRecall={vi.fn()} />);
    const buttons = screen.getAllByRole("button");
    expect(buttons).toHaveLength(SAMPLE.length);
  });

  it("caps rendered lines at MAX_VISIBLE (8) when prompts exceed it", () => {
    const many = Array.from({ length: 12 }, (_, i) => `prompt ${i + 1}`);
    render(<PromptHistoryStack prompts={many} activeIndex={null} onRecall={vi.fn()} />);
    expect(screen.getAllByRole("button")).toHaveLength(8);
  });

  it("shows a 'more above' marker when the window is scrolled", () => {
    // 12 prompts → start=4, so 4 items are above the window
    const many = Array.from({ length: 12 }, (_, i) => `prompt ${i + 1}`);
    render(<PromptHistoryStack prompts={many} activeIndex={null} onRecall={vi.fn()} />);
    expect(screen.getByText("···")).toBeTruthy();
  });

  it("does not show the 'more above' marker when all prompts fit", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={null} onRecall={vi.fn()} />);
    expect(screen.queryByText("···")).toBeNull();
  });

  it("shows prompt text on mouseEnter and hides it on mouseLeave", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={null} onRecall={vi.fn()} />);
    const buttons = screen.getAllByRole("button");

    // Initially not visible
    expect(screen.queryByText("second prompt")).toBeNull();

    // Hover the second line
    fireEvent.mouseEnter(buttons[1]);
    expect(screen.getByText("second prompt")).toBeTruthy();

    // Leave
    fireEvent.mouseLeave(buttons[1]);
    expect(screen.queryByText("second prompt")).toBeNull();
  });

  it("calls onRecall with the correct (full) index when a line is clicked", () => {
    const onRecall = vi.fn();
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={null} onRecall={onRecall} />);
    const buttons = screen.getAllByRole("button");

    fireEvent.click(buttons[0]);
    expect(onRecall).toHaveBeenCalledWith(0);

    fireEvent.click(buttons[2]);
    expect(onRecall).toHaveBeenCalledWith(2);
  });

  it("passes the original (full-array) index to onRecall when the window is scrolled", () => {
    // 12 prompts, window shows indices 4–11; clicking the first visible button → idx 4
    const many = Array.from({ length: 12 }, (_, i) => `prompt ${i + 1}`);
    const onRecall = vi.fn();
    render(<PromptHistoryStack prompts={many} activeIndex={null} onRecall={onRecall} />);

    const buttons = screen.getAllByRole("button");
    fireEvent.click(buttons[0]);
    expect(onRecall).toHaveBeenCalledWith(4);
  });

  it("gives the active line the accent class", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={1} onRecall={vi.fn()} />);
    const buttons = screen.getAllByRole("button");

    // The bar span inside button[1] should have the accent class
    const activeBar = buttons[1].querySelector("span");
    expect(activeBar?.className).toContain("bg-accent");
  });

  it("does not give non-active lines the accent class", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={1} onRecall={vi.fn()} />);
    const buttons = screen.getAllByRole("button");

    const bar0 = buttons[0].querySelector("span");
    const bar2 = buttons[2].querySelector("span");
    expect(bar0?.className).not.toContain("bg-accent");
    expect(bar2?.className).not.toContain("bg-accent");
  });

  it("gives the newest line a brighter dim shade when not active", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={null} onRecall={vi.fn()} />);
    const buttons = screen.getAllByRole("button");

    // Last button (newest) → bg-ink-dim/60
    const newestBar = buttons[2].querySelector("span");
    expect(newestBar?.className).toContain("bg-ink-dim/60");

    // Non-newest, non-active → bg-ink-dim/30
    const olderBar = buttons[0].querySelector("span");
    expect(olderBar?.className).toContain("bg-ink-dim/30");
  });

  it("renders correct aria-labels referencing the full-array position", () => {
    render(<PromptHistoryStack prompts={SAMPLE} activeIndex={null} onRecall={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Recall prompt 1 of 3" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Recall prompt 3 of 3" })).toBeTruthy();
  });
});
