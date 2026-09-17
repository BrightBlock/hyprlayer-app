import { SPLIT_PANE_MIN } from "@/lib/layout";

export type PaneId = string;
export type SplitDir = "row" | "col";
export type SplitPath = ReadonlyArray<"a" | "b">;
export type LayoutNode =
  | { type: "leaf"; paneId: PaneId }
  | { type: "split"; dir: SplitDir; ratio: number; a: LayoutNode; b: LayoutNode };
export type SplitNode = Extract<LayoutNode, { type: "split" }>;
export type Rect = { x: number; y: number; w: number; h: number };
export type Direction = "left" | "right" | "up" | "down";
export type Edge = "left" | "right" | "top" | "bottom";

export const leaf = (paneId: PaneId): LayoutNode => ({ type: "leaf", paneId });

export function leaves(node: LayoutNode): PaneId[] {
  return node.type === "leaf" ? [node.paneId] : [...leaves(node.a), ...leaves(node.b)];
}

export function newPaneId(node: LayoutNode): PaneId {
  const highest = leaves(node).reduce((max, id) => {
    const match = /^p(\d+)$/.exec(id);
    return match ? Math.max(max, Number(match[1])) : max;
  }, 0);
  return `p${highest + 1}`;
}

export function splitLeaf(
  node: LayoutNode,
  target: PaneId,
  dir: SplitDir,
  place: "before" | "after",
  newId: PaneId,
): LayoutNode {
  if (node.type === "leaf") {
    if (node.paneId !== target) return node;
    const added = leaf(newId);
    return {
      type: "split",
      dir,
      ratio: 0.5,
      a: place === "before" ? added : node,
      b: place === "before" ? node : added,
    };
  }
  const a = splitLeaf(node.a, target, dir, place, newId);
  if (a !== node.a) return { ...node, a };
  const b = splitLeaf(node.b, target, dir, place, newId);
  return b === node.b ? node : { ...node, b };
}

export function removeLeaf(
  node: LayoutNode,
  target: PaneId,
): { layout: LayoutNode; heir: PaneId } | null {
  if (node.type === "leaf") return null;
  if (node.a.type === "leaf" && node.a.paneId === target) {
    return { layout: node.b, heir: leaves(node.b)[0] };
  }
  if (node.b.type === "leaf" && node.b.paneId === target) {
    return { layout: node.a, heir: leaves(node.a)[0] };
  }
  const inA = removeLeaf(node.a, target);
  if (inA) return { layout: { ...node, a: inA.layout }, heir: inA.heir };
  const inB = removeLeaf(node.b, target);
  return inB ? { layout: { ...node, b: inB.layout }, heir: inB.heir } : null;
}

export function minExtent(node: LayoutNode, axis: "x" | "y"): number {
  if (node.type === "leaf") return SPLIT_PANE_MIN;
  const sameAxis = (node.dir === "row") === (axis === "x");
  const a = minExtent(node.a, axis);
  const b = minExtent(node.b, axis);
  return sameAxis ? a + b : Math.max(a, b);
}

export function nodeAt(node: LayoutNode, path: SplitPath): LayoutNode | null {
  let current: LayoutNode = node;
  for (const step of path) {
    if (current.type !== "split") return null;
    current = current[step];
  }
  return current;
}

export function setRatio(node: LayoutNode, path: SplitPath, ratio: number): LayoutNode {
  if (path.length === 0) return node.type === "split" ? { ...node, ratio } : node;
  if (node.type === "leaf") return node;
  const [step, ...rest] = path;
  if (step === "a") {
    const a = setRatio(node.a, rest, ratio);
    return a === node.a ? node : { ...node, a };
  }
  const b = setRatio(node.b, rest, ratio);
  return b === node.b ? node : { ...node, b };
}

function splitChildRects(node: SplitNode, rect: Rect): [Rect, Rect] {
  if (node.dir === "row") {
    const aw = rect.w * node.ratio;
    return [{ ...rect, w: aw }, { ...rect, x: rect.x + aw, w: rect.w - aw }];
  }
  const ah = rect.h * node.ratio;
  return [{ ...rect, h: ah }, { ...rect, y: rect.y + ah, h: rect.h - ah }];
}

export function paneRects(
  node: LayoutNode,
  w: number,
  h: number,
): Array<{ paneId: PaneId; rect: Rect }> {
  return paneRectsAt(node, { x: 0, y: 0, w, h });
}

function paneRectsAt(node: LayoutNode, rect: Rect): Array<{ paneId: PaneId; rect: Rect }> {
  if (node.type === "leaf") return [{ paneId: node.paneId, rect }];
  const [aRect, bRect] = splitChildRects(node, rect);
  return [...paneRectsAt(node.a, aRect), ...paneRectsAt(node.b, bRect)];
}

export function splitRects(
  node: LayoutNode,
  w: number,
  h: number,
): Array<{ path: SplitPath; node: SplitNode; rect: Rect }> {
  return splitRectsAt(node, { x: 0, y: 0, w, h }, []);
}

function splitRectsAt(
  node: LayoutNode,
  rect: Rect,
  path: SplitPath,
): Array<{ path: SplitPath; node: SplitNode; rect: Rect }> {
  if (node.type === "leaf") return [];
  const [aRect, bRect] = splitChildRects(node, rect);
  return [
    { path, node, rect },
    ...splitRectsAt(node.a, aRect, [...path, "a"]),
    ...splitRectsAt(node.b, bRect, [...path, "b"]),
  ];
}

export function neighborPane(node: LayoutNode, from: PaneId, dir: Direction): PaneId | null {
  const rects = paneRects(node, 1, 1);
  const f = rects.find((r) => r.paneId === from)?.rect;
  if (!f) return null;
  const horizontal = dir === "left" || dir === "right";

  const candidates = rects
    .filter((r) => r.paneId !== from)
    .filter((r) => {
      const c = r.rect;
      switch (dir) {
        case "right":
          return c.x >= f.x + f.w - 1e-9;
        case "left":
          return c.x + c.w <= f.x + 1e-9;
        case "down":
          return c.y >= f.y + f.h - 1e-9;
        case "up":
          return c.y + c.h <= f.y + 1e-9;
      }
    })
    .map((r) => {
      const c = r.rect;
      const gap =
        dir === "right"
          ? c.x - (f.x + f.w)
          : dir === "left"
            ? f.x - (c.x + c.w)
            : dir === "down"
              ? c.y - (f.y + f.h)
              : f.y - (c.y + c.h);
      const overlap = horizontal
        ? Math.min(c.y + c.h, f.y + f.h) - Math.max(c.y, f.y)
        : Math.min(c.x + c.w, f.x + f.w) - Math.max(c.x, f.x);
      return { paneId: r.paneId, gap, overlap, rect: c };
    })
    .filter((r) => r.overlap > 1e-9);

  candidates.sort((a, b) => {
    if (Math.abs(a.gap - b.gap) > 1e-9) return a.gap - b.gap;
    if (Math.abs(a.overlap - b.overlap) > 1e-9) return b.overlap - a.overlap;
    return horizontal ? a.rect.y - b.rect.y : a.rect.x - b.rect.x;
  });
  return candidates[0]?.paneId ?? null;
}

export function edgeZone(
  rect: { left: number; top: number; width: number; height: number },
  x: number,
  y: number,
  band = 0.25,
): Edge | null {
  const dx = x - rect.left;
  const dy = y - rect.top;
  if (dx < 0 || dy < 0 || dx > rect.width || dy > rect.height) return null;
  const distances: Array<[Edge, number]> = [
    ["left", rect.width > 0 ? dx / rect.width : 0],
    ["right", rect.width > 0 ? (rect.width - dx) / rect.width : 0],
    ["top", rect.height > 0 ? dy / rect.height : 0],
    ["bottom", rect.height > 0 ? (rect.height - dy) / rect.height : 0],
  ];
  const [edge, nearest] = distances.reduce((min, cur) => (cur[1] < min[1] ? cur : min));
  return nearest < band ? edge : null;
}
