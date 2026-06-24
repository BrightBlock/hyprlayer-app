import { describe, expect, it } from "vitest";

import type { WorkstreamView } from "./bindings";
import { selectPinnedViews, togglePinId } from "./pinStore";

function makeView(id: string, projectPath = "/project"): WorkstreamView {
  return {
    item: {
      id,
      projectPath,
      title: `Workstream ${id}`,
      slug: id,
      branch: `feature/${id}`,
      worktreePath: `/worktrees/${id}`,
      baseBranch: "main",
      worktreeMode: "isolated",
      stage: "general",
      status: "active",
      prNumber: null,
      createdAt: null,
      updatedAt: null,
      archivedAt: null,
    },
    git: {
      exists: true,
      branch: `feature/${id}`,
      dirty: 0,
      additions: 0,
      deletions: 0,
      untracked: 0,
      ahead: 0,
      behind: 0,
      thoughts: "ok",
    },
    seed: null,
  };
}

describe("togglePinId", () => {
  it("appends an id that isn't pinned, in pin order", () => {
    expect(togglePinId([], "a")).toEqual(["a"]);
    expect(togglePinId(["a"], "b")).toEqual(["a", "b"]);
  });

  it("removes an id that is already pinned, keeping the rest in order", () => {
    expect(togglePinId(["a", "b", "c"], "b")).toEqual(["a", "c"]);
  });

  it("does not mutate the input array", () => {
    const ids = ["a", "b"];
    togglePinId(ids, "c");
    togglePinId(ids, "a");
    expect(ids).toEqual(["a", "b"]);
  });
});

describe("selectPinnedViews", () => {
  it("resolves ids to views in pin order, not list order", () => {
    const views = [makeView("a"), makeView("b"), makeView("c")];
    const result = selectPinnedViews(["c", "a"], views);
    expect(result.map((v) => v.item.id)).toEqual(["c", "a"]);
  });

  it("drops ids that no longer resolve (archived / deleted worktrees)", () => {
    const views = [makeView("a"), makeView("c")];
    const result = selectPinnedViews(["a", "ghost", "c"], views);
    expect(result.map((v) => v.item.id)).toEqual(["a", "c"]);
  });

  it("returns an empty list when nothing is pinned", () => {
    expect(selectPinnedViews([], [makeView("a")])).toEqual([]);
  });
});
