import { describe, expect, it } from "vitest";

import { nestWorkstreams, toggleCollapsedPath } from "./projectTreeStore";

import type { WorkstreamView } from "@/lib/bindings";

describe("toggleCollapsedPath", () => {
  it("collapses a path that is currently expanded", () => {
    expect(toggleCollapsedPath([], "/a")).toEqual(["/a"]);
    expect(toggleCollapsedPath(["/a"], "/b")).toEqual(["/a", "/b"]);
  });

  it("expands a path that is currently collapsed, keeping the rest", () => {
    expect(toggleCollapsedPath(["/a", "/b", "/c"], "/b")).toEqual(["/a", "/c"]);
  });

  it("does not mutate the input array", () => {
    const paths = ["/a", "/b"];
    toggleCollapsedPath(paths, "/c");
    toggleCollapsedPath(paths, "/a");
    expect(paths).toEqual(["/a", "/b"]);
  });

});

function ws(id: string, parentWorkstreamId?: string): WorkstreamView {
  return { item: { id, parentWorkstreamId } } as WorkstreamView;
}

function shape(rows: ReturnType<typeof nestWorkstreams>) {
  return rows.map(({ view, depth, childCount }) => [view.item.id, depth, childCount]);
}

describe("nestWorkstreams", () => {
  it("puts each child directly beneath its parent, one level in", () => {
    const views = [ws("a"), ws("b"), ws("a-child", "a"), ws("grandchild", "a-child")];
    expect(shape(nestWorkstreams(views, []))).toEqual([
      ["a", 0, 1],
      ["a-child", 1, 1],
      ["grandchild", 2, 0],
      ["b", 0, 0],
    ]);
  });

  it("hides a collapsed parent's children and keeps their count on the parent", () => {
    const views = [ws("a"), ws("one", "a"), ws("two", "a"), ws("b")];
    expect(shape(nestWorkstreams(views, ["a"]))).toEqual([
      ["a", 0, 2],
      ["b", 0, 0],
    ]);
    expect(shape(nestWorkstreams(views, []))).toEqual([
      ["a", 0, 2],
      ["one", 1, 0],
      ["two", 1, 0],
      ["b", 0, 0],
    ]);
  });

  it("shows a child whose parent is not in the list at the top level", () => {
    const views = [ws("orphan", "archived-parent"), ws("b")];
    expect(shape(nestWorkstreams(views, []))).toEqual([
      ["orphan", 0, 0],
      ["b", 0, 0],
    ]);
  });
});
