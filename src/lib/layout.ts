
export const PROJECT_SIDEBAR_MIN = 180;
export const PROJECT_SIDEBAR_MAX = 360;
export const NAV_RAIL_WIDTH = 48;

export const NARROW_WIDTH = 900;
export const COMPACT_WIDTH = 640;
export const DRAWER_PEEK = 72;
export const SPLIT_HANDLE_WIDTH = 8;
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

export function splitPaneFloor(rowWidth: number): number {
  return Math.min(SPLIT_PANE_MIN, Math.floor((rowWidth - SPLIT_HANDLE_WIDTH) / 2));
}

export const TREE_INDENT_STEP = 14;
export function treeIndent(depth: number, base = 8): number {
  return base + depth * TREE_INDENT_STEP;
}
