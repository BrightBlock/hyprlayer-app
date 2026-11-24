
export const PROJECT_SIDEBAR_MIN = 180;
export const PROJECT_SIDEBAR_DEFAULT = 268;
export const PROJECT_SIDEBAR_MAX = 360;
export const NAV_RAIL_WIDTH = 48;
export const STATUS_BAR_HEIGHT = 34;
const TITLEBAR_INSET_LIGHTS = 92;
export const TRAFFIC_LIGHTS = { x: 18, y: 20 };

// Native lights sit at TRAFFIC_LIGHTS * zoom; the gap after them stays a fixed native width.
export function titlebarInsetLights(zoom: number): number {
  return TRAFFIC_LIGHTS.x + (TITLEBAR_INSET_LIGHTS - TRAFFIC_LIGHTS.x) / zoom;
}
export const TITLEBAR_INSET = 12;

export const NARROW_WIDTH = 900;
export const COMPACT_WIDTH = 640;
export const DRAWER_PEEK = 72;
const SPLIT_HANDLE_WIDTH = 8;
export const SPLIT_PANE_MIN = 280;

export function isNarrow(viewport: number): boolean {
  return viewport < NARROW_WIDTH;
}

export function isCompact(viewport: number): boolean {
  return viewport < COMPACT_WIDTH;
}

export function projectSidebarWidth(dragged: number, viewport: number): number {
  return isNarrow(viewport) ? PROJECT_SIDEBAR_MIN : dragged;
}

export function drawerWidth(preferred: number, available: number): number {
  return Math.min(preferred, Math.max(0, available - DRAWER_PEEK));
}

export function splitPaneFloor(size: number, minA = SPLIT_PANE_MIN, minB = SPLIT_PANE_MIN): number {
  return Math.min(minA, Math.floor(((size - SPLIT_HANDLE_WIDTH) * minA) / (minA + minB)));
}

const SPLIT_PANE_HARD_MIN = 160;

export function canSplitExtent(extent: number): boolean {
  return Math.floor((extent - SPLIT_HANDLE_WIDTH) / 2) >= SPLIT_PANE_HARD_MIN;
}

const TREE_INDENT_STEP = 14;
export function treeIndent(depth: number, base = 8): number {
  return base + depth * TREE_INDENT_STEP;
}
