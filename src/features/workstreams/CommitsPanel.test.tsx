// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CommitEntry, GitFileEntry } from "@/lib/bindings";
import { formatRelativeTime } from "@/lib/format";
import { CommitsPanel } from "./CommitsPanel";

const q = vi.hoisted(() => ({ useCommitFiles: vi.fn() }));
vi.mock("@/lib/queries", () => q);

const NOW = Date.UTC(2026, 6, 10, 12, 0, 0);

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

const SHA = "a1b2c3d4e5f6789012345678901234567890abcd";

function commit(overrides: Partial<CommitEntry> = {}): CommitEntry {
  return {
    sha: SHA,
    subject: "add the commits tab",
    author: "jt",
    timeMs: NOW - 2 * 60 * 60 * 1000,
    ...overrides,
  };
}

function file(path: string, change: string, oldPath: string | null = null): GitFileEntry {
  return { path, change, oldPath };
}

function filesResult(data: GitFileEntry[] | undefined, overrides: Record<string, unknown> = {}) {
  return { data, isLoading: false, isError: false, error: null, ...overrides };
}

describe("CommitsPanel", () => {
  it("renders sha, subject, author and relative time; a collapsed row never fetches its files", () => {
    q.useCommitFiles.mockReturnValue(filesResult(undefined));
    render(<CommitsPanel id="ws1" commits={[commit()]} onOpenDiff={vi.fn()} />);

    expect(screen.getByText("a1b2c3d")).toBeDefined();
    expect(screen.getByText("add the commits tab")).toBeDefined();
    expect(screen.getByText("jt")).toBeDefined();
    expect(screen.getByText(formatRelativeTime(commit().timeMs, NOW))).toBeDefined();

    expect(q.useCommitFiles).toHaveBeenCalledWith("ws1", SHA, false);
  });

  it("fetches and lists files once expanded", () => {
    q.useCommitFiles.mockReturnValue(
      filesResult([file("src/a.ts", "modified"), file("src/b.ts", "added")]),
    );
    render(<CommitsPanel id="ws1" commits={[commit()]} onOpenDiff={vi.fn()} />);

    fireEvent.click(screen.getByText("add the commits tab"));

    expect(q.useCommitFiles).toHaveBeenCalledWith("ws1", SHA, true);
    expect(screen.getByText("src/a.ts")).toBeDefined();
    expect(screen.getByText("src/b.ts")).toBeDefined();
  });

  it("renders a renamed file's glyph, not the unknown fallback", () => {
    q.useCommitFiles.mockReturnValue(filesResult([file("src/new-name.ts", "renamed")]));
    render(<CommitsPanel id="ws1" commits={[commit()]} onOpenDiff={vi.fn()} />);

    fireEvent.click(screen.getByText("add the commits tab"));

    expect(screen.getByTitle("renamed").textContent).toBe("R");
  });

  it("shows a rename as old → new but opens the diff at the new path", () => {
    q.useCommitFiles.mockReturnValue(
      filesResult([file("src/new-name.ts", "renamed", "src/old-name.ts")]),
    );
    const onOpenDiff = vi.fn();
    render(<CommitsPanel id="ws1" commits={[commit()]} onOpenDiff={onOpenDiff} />);

    fireEvent.click(screen.getByText("add the commits tab"));
    const row = screen.getByText("src/old-name.ts → src/new-name.ts");

    // The old path is re-derived backend-side from the commit's own diff, so
    // the click carries the new path and the full sha, exactly as any other row.
    fireEvent.click(row);
    expect(onOpenDiff).toHaveBeenCalledWith("src/new-name.ts", false, SHA);
  });

  it("opens the sha-addressed diff with the full sha when a file is clicked", () => {
    q.useCommitFiles.mockReturnValue(filesResult([file("src/a.ts", "modified")]));
    const onOpenDiff = vi.fn();
    render(<CommitsPanel id="ws1" commits={[commit()]} onOpenDiff={onOpenDiff} />);

    fireEvent.click(screen.getByText("add the commits tab"));
    fireEvent.click(screen.getByText("src/a.ts"));

    expect(onOpenDiff).toHaveBeenCalledWith("src/a.ts", false, SHA);
  });
});
