// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BackgroundTasksIndicator } from "./BackgroundTasksIndicator";
import type { LiveBackgroundTask } from "./sessionStore";

afterEach(cleanup);

function running(toolUseId = "tu-1", startedAt: number | null = Date.now()): LiveBackgroundTask {
  return {
    toolUseId,
    taskId: `task-${toolUseId}`,
    label: `sleep 999 (${toolUseId})`,
    command: "sleep 999",
    status: "running",
    startedAt,
  };
}

function awaiting(toolUseId = "tu-2"): LiveBackgroundTask {
  return { ...running(toolUseId, Date.now()), status: "awaiting" };
}

/** Open the indicator popover by clicking its toolbar button. */
function openPopover() {
  fireEvent.click(screen.getByRole("button", { name: /background task/ }));
}

describe("BackgroundTasksIndicator", () => {
  it("renders nothing when there are no tasks", () => {
    const { container } = render(
      <BackgroundTasksIndicator tasks={[]} onJumpTo={vi.fn()} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("shows the task count and a singular aria-label for one task", () => {
    render(<BackgroundTasksIndicator tasks={[running()]} onJumpTo={vi.fn()} />);
    expect(screen.getByRole("button", { name: "1 background task" })).toBeTruthy();
    expect(screen.getByText("1")).toBeTruthy();
  });

  it("shows a plural aria-label and the count for multiple tasks", () => {
    render(
      <BackgroundTasksIndicator
        tasks={[running("tu-1"), running("tu-2")]}
        onJumpTo={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "2 background tasks" })).toBeTruthy();
    expect(screen.getByText("2")).toBeTruthy();
  });

  it("renders a pulsing icon in the button", () => {
    render(<BackgroundTasksIndicator tasks={[running()]} onJumpTo={vi.fn()} />);
    const svg = screen
      .getByRole("button", { name: /background task/ })
      .querySelector("svg");
    expect(svg?.classList.contains("animate-work-pulse")).toBe(true);
  });

  it("keeps the popover closed by default, task rows are not visible", () => {
    render(<BackgroundTasksIndicator tasks={[running()]} onJumpTo={vi.fn()} />);
    expect(screen.queryByText(/sleep 999/)).toBeNull();
  });

  it("opens the popover on click, revealing each task row", () => {
    render(<BackgroundTasksIndicator tasks={[running("tu-1")]} onJumpTo={vi.fn()} />);
    openPopover();
    expect(screen.getByText("sleep 999 (tu-1)")).toBeTruthy();
  });

  it("closes the popover when the button is clicked again", () => {
    render(<BackgroundTasksIndicator tasks={[running("tu-1")]} onJumpTo={vi.fn()} />);
    openPopover();
    expect(screen.getByText("sleep 999 (tu-1)")).toBeTruthy();
    openPopover();
    expect(screen.queryByText("sleep 999 (tu-1)")).toBeNull();
  });

  it("shows a 'running' hint for a running task", () => {
    render(<BackgroundTasksIndicator tasks={[running()]} onJumpTo={vi.fn()} />);
    openPopover();
    expect(screen.getByText("running")).toBeTruthy();
  });

  it("shows a 'waiting…' hint for an awaiting task", () => {
    render(<BackgroundTasksIndicator tasks={[awaiting()]} onJumpTo={vi.fn()} />);
    openPopover();
    expect(screen.getByText("waiting…")).toBeTruthy();
  });

  it("fires onJumpTo with the task's toolUseId and closes the popover on row click", () => {
    const onJumpTo = vi.fn();
    render(
      <BackgroundTasksIndicator tasks={[running("tu-42")]} onJumpTo={onJumpTo} />,
    );
    openPopover();
    fireEvent.click(screen.getByTitle("Jump to transcript"));
    expect(onJumpTo).toHaveBeenCalledWith("tu-42");
    // Popover closes after a jump.
    expect(screen.queryByText("sleep 999 (tu-42)")).toBeNull();
  });

  it("closes the popover when the outside overlay is clicked", () => {
    render(<BackgroundTasksIndicator tasks={[running("tu-1")]} onJumpTo={vi.fn()} />);
    openPopover();
    expect(screen.getByText("sleep 999 (tu-1)")).toBeTruthy();
    // The Popover portals its backdrop onto document.body, not into this
    // component's own container.
    const overlay = document.body.querySelector('div.fixed.inset-0[aria-hidden="true"]');
    expect(overlay).toBeTruthy();
    fireEvent.click(overlay!);
    expect(screen.queryByText("sleep 999 (tu-1)")).toBeNull();
  });

  it("shows an elapsed timer for a task with a startedAt", () => {
    const fiveSecondsAgo = Date.now() - 5000;
    render(
      <BackgroundTasksIndicator
        tasks={[running("tu-1", fiveSecondsAgo)]}
        onJumpTo={vi.fn()}
      />,
    );
    openPopover();
    expect(screen.getByText(/^\d+\.\d+s$/)).toBeTruthy();
  });

  it("omits the elapsed timer when startedAt is null", () => {
    render(
      <BackgroundTasksIndicator tasks={[running("tu-1", null)]} onJumpTo={vi.fn()} />,
    );
    openPopover();
    expect(screen.queryByText(/^\d+\.\d+s$/)).toBeNull();
  });

  it("disappears entirely once the last task ends", () => {
    const { container, rerender } = render(
      <BackgroundTasksIndicator tasks={[running("tu-1")]} onJumpTo={vi.fn()} />,
    );
    openPopover();
    expect(screen.getByText("sleep 999 (tu-1)")).toBeTruthy();
    rerender(<BackgroundTasksIndicator tasks={[]} onJumpTo={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });
});
