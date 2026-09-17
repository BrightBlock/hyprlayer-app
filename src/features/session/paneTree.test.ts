import { describe, expect, it } from "vitest";

import {
  edgeZone,
  leaf,
  leaves,
  minExtent,
  neighborPane,
  newPaneId,
  nodeAt,
  paneRects,
  removeLeaf,
  setRatio,
  splitLeaf,
  splitRects,
  type LayoutNode,
  type SplitDir,
  type SplitNode,
} from "./paneTree";

const split = (dir: SplitDir, ratio: number, a: LayoutNode, b: LayoutNode): SplitNode => ({
  type: "split",
  dir,
  ratio,
  a,
  b,
});

describe("leaves", () => {
  it("returns tree order: a before b, on a 3-leaf mixed tree", () => {
    const tree = split("row", 0.5, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));
    expect(leaves(tree)).toEqual(["p1", "p2", "p3"]);
  });
});

describe("newPaneId", () => {
  it("increments the only leaf's suffix", () => {
    expect(newPaneId(leaf("p1"))).toBe("p2");
  });

  it("skips past the highest suffix rather than counting leaves", () => {
    const tree = split("row", 0.5, leaf("p1"), leaf("p5"));
    expect(newPaneId(tree)).toBe("p6");
  });
});

describe("splitLeaf", () => {
  it("places the new leaf after and before the target on a row split", () => {
    expect(splitLeaf(leaf("p1"), "p1", "row", "after", "p2")).toEqual(
      split("row", 0.5, leaf("p1"), leaf("p2")),
    );
    expect(splitLeaf(leaf("p1"), "p1", "row", "before", "p2")).toEqual(
      split("row", 0.5, leaf("p2"), leaf("p1")),
    );
  });

  it("places the new leaf after and before the target on a col split", () => {
    expect(splitLeaf(leaf("p1"), "p1", "col", "after", "p2")).toEqual(
      split("col", 0.5, leaf("p1"), leaf("p2")),
    );
    expect(splitLeaf(leaf("p1"), "p1", "col", "before", "p2")).toEqual(
      split("col", 0.5, leaf("p2"), leaf("p1")),
    );
  });

  it("splits a nested target in place", () => {
    const tree = split("row", 0.5, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));
    const result = splitLeaf(tree, "p3", "row", "after", "p4");
    expect(result).toEqual(
      split(
        "row",
        0.5,
        leaf("p1"),
        split("col", 0.5, leaf("p2"), split("row", 0.5, leaf("p3"), leaf("p4"))),
      ),
    );
  });

  it("returns the same reference for an unknown target", () => {
    const tree = split("row", 0.5, leaf("p1"), leaf("p2"));
    expect(splitLeaf(tree, "p99", "row", "after", "p3")).toBe(tree);
  });
});

describe("removeLeaf", () => {
  it("returns null for the root leaf", () => {
    expect(removeLeaf(leaf("p1"), "p1")).toBeNull();
  });

  it("returns null for an absent target", () => {
    const tree = split("row", 0.5, leaf("p1"), leaf("p2"));
    expect(removeLeaf(tree, "p99")).toBeNull();
  });

  const tree = split("row", 0.5, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));

  it("removing p1 collapses to the sibling subtree, heir p2", () => {
    const result = removeLeaf(tree, "p1");
    expect(result?.heir).toBe("p2");
    expect(result?.layout).toEqual(split("col", 0.5, leaf("p2"), leaf("p3")));
  });

  it("removing p2 collapses its parent split, heir p3", () => {
    const result = removeLeaf(tree, "p2");
    expect(result?.heir).toBe("p3");
    expect(result?.layout).toEqual(split("row", 0.5, leaf("p1"), leaf("p3")));
  });

  it("removing p3 collapses its parent split, heir p2", () => {
    const result = removeLeaf(tree, "p3");
    expect(result?.heir).toBe("p2");
    expect(result?.layout).toEqual(split("row", 0.5, leaf("p1"), leaf("p2")));
  });
});

describe("nodeAt", () => {
  const tree = split("row", 0.3, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));

  it("returns the node at a path by reference, and null past a leaf", () => {
    expect(nodeAt(tree, [])).toBe(tree);
    expect(nodeAt(tree, ["b"])).toBe(tree.b);
    expect(nodeAt(tree, ["a", "a"])).toBeNull();
  });

  it("still names a captured split when an unrelated subtree is reshaped", () => {
    const reshaped = splitLeaf(tree, "p1", "row", "after", "p4");
    expect(nodeAt(reshaped, ["b"])).toBe(tree.b);
  });

  it("stops naming a captured split once a removal promotes the tree under it", () => {
    const removed = removeLeaf(tree, "p1");
    expect(nodeAt(removed!.layout, ["b"])).not.toBe(tree.b);
  });
});

describe("setRatio", () => {
  const tree = split("row", 0.3, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));

  it("sets the ratio at a nested path", () => {
    expect(setRatio(tree, ["b"], 0.7)).toEqual(
      split("row", 0.3, leaf("p1"), split("col", 0.7, leaf("p2"), leaf("p3"))),
    );
  });

  it("returns the same reference for a path that doesn't reach a split", () => {
    expect(setRatio(tree, ["a", "a"], 0.7)).toBe(tree);
  });
});

describe("minExtent", () => {
  it("sums same-axis children and takes the max across the axis", () => {
    const tree = split("row", 0.5, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));
    expect(minExtent(tree, "x")).toBe(560);
    expect(minExtent(tree, "y")).toBe(560);
  });

  it("sums nested same-direction splits along their shared axis", () => {
    const tree = split("row", 0.5, leaf("p1"), split("row", 0.5, leaf("p2"), leaf("p3")));
    expect(minExtent(tree, "x")).toBe(840);
    expect(minExtent(tree, "y")).toBe(280);
  });
});

describe("paneRects", () => {
  it("computes exact rects for a mixed tree", () => {
    const tree = split("row", 0.3, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));
    expect(paneRects(tree, 1000, 600)).toEqual([
      { paneId: "p1", rect: { x: 0, y: 0, w: 300, h: 600 } },
      { paneId: "p2", rect: { x: 300, y: 0, w: 700, h: 300 } },
      { paneId: "p3", rect: { x: 300, y: 300, w: 700, h: 300 } },
    ]);
  });
});

describe("splitRects", () => {
  it("returns the root path and a nested path with their boxes", () => {
    const tree = split("row", 0.3, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));
    expect(splitRects(tree, 1000, 600)).toEqual([
      { path: [], node: tree, rect: { x: 0, y: 0, w: 1000, h: 600 } },
      { path: ["b"], node: tree.b, rect: { x: 300, y: 0, w: 700, h: 600 } },
    ]);
  });
});

describe("neighborPane", () => {
  const tree = split("row", 0.5, leaf("p1"), split("col", 0.5, leaf("p2"), leaf("p3")));

  it("breaks an overlap tie to the right of p1 by smaller y", () => {
    expect(neighborPane(tree, "p1", "right")).toBe("p2");
  });

  it("finds the pane to the left of p3", () => {
    expect(neighborPane(tree, "p3", "left")).toBe("p1");
  });

  it("finds the pane below p2", () => {
    expect(neighborPane(tree, "p2", "down")).toBe("p3");
  });

  it("returns null with no candidate on that side", () => {
    expect(neighborPane(tree, "p1", "left")).toBeNull();
  });
});

describe("edgeZone", () => {
  const rect = { left: 0, top: 0, width: 100, height: 200 };

  it("returns null outside the rect", () => {
    expect(edgeZone(rect, -5, 50)).toBeNull();
  });

  it("returns null in the centre", () => {
    expect(edgeZone(rect, 50, 100)).toBeNull();
  });

  it("returns the nearest edge close to each side", () => {
    expect(edgeZone(rect, 5, 100)).toBe("left");
    expect(edgeZone(rect, 95, 100)).toBe("right");
    expect(edgeZone(rect, 50, 10)).toBe("top");
    expect(edgeZone(rect, 50, 190)).toBe("bottom");
  });

  it("picks the nearer edge at a corner", () => {
    expect(edgeZone(rect, 5, 30)).toBe("left");
  });
});
