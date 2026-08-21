import { describe, expect, it } from "vitest";

import {
  COMPACT_WIDTH,
  drawerWidth,
  isCompact,
  isNarrow,
  NARROW_WIDTH,
  projectSidebarWidth,
  PROJECT_SIDEBAR_MIN,
  splitPaneFloor,
  treeIndent,
} from "./layout";

describe("isNarrow", () => {
  it("both sides of the NARROW_WIDTH boundary", () => {
    expect(isNarrow(NARROW_WIDTH - 1)).toBe(true);
    expect(isNarrow(899)).toBe(true);
    expect(isNarrow(NARROW_WIDTH)).toBe(false);
    expect(isNarrow(900)).toBe(false);
  });
});

describe("isCompact", () => {
  it("both sides of the COMPACT_WIDTH boundary", () => {
    expect(isCompact(COMPACT_WIDTH - 1)).toBe(true);
    expect(isCompact(639)).toBe(true);
    expect(isCompact(COMPACT_WIDTH)).toBe(false);
    expect(isCompact(640)).toBe(false);
  });
});

describe("projectSidebarWidth", () => {
  it("passes the dragged width through above the narrow tier", () => {
    expect(projectSidebarWidth(360, 1280)).toBe(360);
    expect(projectSidebarWidth(240, 900)).toBe(240); // pass-through at the boundary
  });

  it("clamps to PROJECT_SIDEBAR_MIN on the narrow tier, a non-default dragged", () => {
    // width so a stub returning `dragged` cannot pass.
    expect(projectSidebarWidth(240, 899)).toBe(PROJECT_SIDEBAR_MIN);
    expect(projectSidebarWidth(360, 701)).toBe(PROJECT_SIDEBAR_MIN);
  });
});

describe("drawerWidth", () => {
  it("passes the preferred width through when it's under the cap", () => {
    expect(drawerWidth(420, 1280)).toBe(420);
    expect(drawerWidth(700, 1280)).toBe(700);
    expect(drawerWidth(420, 653)).toBe(420); // under the cap: 653 - 72 = 581
  });

  it("clamps to the containing box less DRAWER_PEEK above that", () => {
    expect(drawerWidth(700, 653)).toBe(581); // 653 - 72
    expect(drawerWidth(700, 352)).toBe(280); // 352 - 72
  });

  it("measures the containing box, not the viewport", () => {
    // The regression: a 640px window whose ProjectPage pane is 412 (640 - 48
    // rail - 180 clamped sidebar). Sizing the drawer off the window yields 520
    // inside a 412px box, clipped, with no peek strip left to dismiss it.
    expect(drawerWidth(700, 412)).toBe(340);
    expect(drawerWidth(700, 412)).toBeLessThan(412);
  });

  it("floors at 0 rather than going negative on a box smaller than the peek", () => {
    expect(drawerWidth(700, 40)).toBe(0);
    expect(drawerWidth(700, 0)).toBe(0);
  });
});

describe("splitPaneFloor", () => {
  it("passes SPLIT_PANE_MIN through when the row can hold two", () => {
    expect(splitPaneFloor(900)).toBe(280); // floor(892/2) = 446 > 280
    expect(splitPaneFloor(568)).toBe(280); // exact boundary: floor(560/2) = 280
  });

  it("clamps to half the row (less the handle) when it can't", () => {
    // 473 is the real narrow content width: 701 - 48 (rail) - 180 (clamped
    // project sidebar), the drag handle gated off while narrow.
    expect(splitPaneFloor(473)).toBe(232); // floor((473-8)/2)
  });
});

describe("treeIndent", () => {
  it("defaults to an 8px base", () => {
    expect(treeIndent(0)).toBe(8);
    expect(treeIndent(1)).toBe(22);
  });

  it("steps by 14px per depth level", () => {
    expect(treeIndent(0, 6)).toBe(6);
    expect(treeIndent(1, 6)).toBe(20);
    expect(treeIndent(2, 6)).toBe(34);
  });

  it("takes a caller-supplied base, since it encodes a real layout fact", () => {
    expect(treeIndent(0, 0)).toBe(0);
    expect(treeIndent(1, 0)).toBe(14); // AgentList's flat 0-or-14
  });
});
