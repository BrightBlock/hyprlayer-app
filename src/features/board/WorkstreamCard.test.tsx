// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkstreamView } from "../../lib/bindings";
import { WorkstreamCard } from "./WorkstreamCard";

// No vitest globals -> RTL can't register its own auto-cleanup; without this,
// a previous test's mounted copy steals the next test's queries and clicks.
afterEach(cleanup);

function makeView(over: Partial<{ prNumber: number | null }> = {}): WorkstreamView {
  return {
    item: {
      id: "ws-1",
      projectPath: "/project",
      title: "My test workstream",
      slug: "my-test",
      branch: "feature/my-test",
      worktreePath: "/worktrees/my-test",
      baseBranch: "main",
      worktreeMode: "isolated",
      stage: "implement",
      status: "active",
      prNumber: over.prNumber !== undefined ? over.prNumber : null,
      createdAt: null,
      updatedAt: null,
      archivedAt: null,
    },
    git: {
      exists: true,
      branch: "feature/my-test",
      dirty: 0,
      additions: 0,
      deletions: 0,
      untracked: 0,
      ahead: 0,
      behind: 0,
      thoughts: "ok" as const,
    },
    seed: null,
  };
}

describe("WorkstreamCard", () => {
  it("renders the workstream title", () => {
    render(<WorkstreamCard view={makeView()} onOpen={vi.fn()} />);
    expect(screen.getByText("My test workstream")).toBeTruthy();
  });

  it("renders the branch name", () => {
    render(<WorkstreamCard view={makeView()} onOpen={vi.fn()} />);
    expect(screen.getByText("feature/my-test")).toBeTruthy();
  });

  it("shows no PR badge when prNumber is null", () => {
    render(<WorkstreamCard view={makeView({ prNumber: null })} onOpen={vi.fn()} />);
    expect(screen.queryByText(/PR #/)).toBeNull();
  });

  it("shows a PR badge when prNumber is set", () => {
    render(<WorkstreamCard view={makeView({ prNumber: 42 })} onOpen={vi.fn()} />);
    expect(screen.getByText("PR #42")).toBeTruthy();
  });

  it("calls onOpen with the workstream id on click", () => {
    const onOpen = vi.fn();
    render(<WorkstreamCard view={makeView()} onOpen={onOpen} />);
    fireEvent.click(screen.getByRole("button"));
    expect(onOpen).toHaveBeenCalledWith("ws-1");
  });

  it("calls onOpen with the workstream id on Enter key", () => {
    const onOpen = vi.fn();
    render(<WorkstreamCard view={makeView()} onOpen={onOpen} />);
    fireEvent.keyDown(screen.getByRole("button"), { key: "Enter" });
    expect(onOpen).toHaveBeenCalledWith("ws-1");
  });

  it("calls onOpen with the workstream id on Space key", () => {
    const onOpen = vi.fn();
    render(<WorkstreamCard view={makeView()} onOpen={onOpen} />);
    fireEvent.keyDown(screen.getByRole("button"), { key: " " });
    expect(onOpen).toHaveBeenCalledWith("ws-1");
  });

  it("does not call onOpen on other key presses", () => {
    const onOpen = vi.fn();
    render(<WorkstreamCard view={makeView()} onOpen={onOpen} />);
    fireEvent.keyDown(screen.getByRole("button"), { key: "Tab" });
    expect(onOpen).not.toHaveBeenCalled();
  });
});
