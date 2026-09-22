import { describe, expect, it } from "vitest";

import { toggleCollapsedPath } from "./projectTreeStore";

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

  it("treats an unknown path as expanded, so new projects open by default", () => {
    expect(toggleCollapsedPath(["/a"], "/fresh")).toEqual(["/a", "/fresh"]);
  });
});
