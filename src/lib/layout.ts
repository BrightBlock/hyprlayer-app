/**
 * Shared layout vocabulary for the shell: the column widths and the tier
 * predicates that clamp them at a narrow viewport (see `useViewport`). Pure —
 * no imports — so it's trivially unit-testable and can't violate the src/lib
 * layering rules (ADR-0003).
 */

// moved from Root.tsx:127
export const PROJECT_SIDEBAR_MIN = 180;
// moved from Root.tsx:128
export const PROJECT_SIDEBAR_MAX = 360;
// moved from Root.tsx:132. The activity-bar nav rail that sits left of the
// project panel (fixed width). 48 holds the 32px icon squares with 8px of
// breathing room each side — the same ratio VSCode's activity bar uses, and
// 8px tighter than the original 56.
export const NAV_RAIL_WIDTH = 48;

/** Below this viewport width the shell drops to its narrow tier: the project
 *  sidebar is clamped to its minimum and a shown side panel becomes an overlay
 *  drawer rather than an in-flow column. Sized so a Hyprland half-tile on a
 *  1440-logical-px display (~701) lands well inside it, and a 1280 default
 *  window never does. */
export const NARROW_WIDTH = 900;
/** Below this, even a clamped project column costs more than the content pane
 *  can spare — the panel is drawer-only. */
export const COMPACT_WIDTH = 640;
/** How much of the content behind an overlay drawer stays visible, so the
 *  drawer reads as an overlay rather than a full-width takeover and there is
 *  always somewhere to click to dismiss it (Root renders a scrim over the
 *  content behind the drawer; this is what guarantees the scrim is reachable).
 *  Named because three call sites and the unit tests all derive from it. */
export const DRAWER_PEEK = 72;
/** The agent split's separator (`w-2`, ProjectPage.tsx:1717) — subtracted
 *  before halving the row so two floors plus the handle actually fit. */
export const SPLIT_HANDLE_WIDTH = 8;
/** The comfortable per-side floor for the agent split (today's hard-coded 280). */
export const SPLIT_PANE_MIN = 280;

/** Whether `viewport` is below the narrow tier. */
export function isNarrow(viewport: number): boolean {
  return viewport < NARROW_WIDTH;
}

/** Whether `viewport` is below the compact tier (a strict subset of narrow). */
export function isCompact(viewport: number): boolean {
  return viewport < COMPACT_WIDTH;
}

/** The content-pane equivalents of NARROW_WIDTH / COMPACT_WIDTH, for panes that
 *  must tier off their OWN width rather than the window's. The shell tiers
 *  above bake in the columns to the pane's left (the nav rail, and — at narrow,
 *  where it is still an in-flow column — the clamped project sidebar), so
 *  testing a pane's width against them would drop content a whole sidebar's
 *  worth too late. Each is the pane width at its shell tier's boundary:
 *  900 − 48 − 180 at narrow, and 640 − 48 at compact (where the project
 *  sidebar has become a drawer and stopped taking room in flow). */
export const CONTENT_NARROW_WIDTH = NARROW_WIDTH - NAV_RAIL_WIDTH - PROJECT_SIDEBAR_MIN;
export const CONTENT_COMPACT_WIDTH = COMPACT_WIDTH - NAV_RAIL_WIDTH;

/** Whether a content pane of `paneWidth` is below the narrow tier. */
export function isContentNarrow(paneWidth: number): boolean {
  return paneWidth < CONTENT_NARROW_WIDTH;
}

/** Whether a content pane of `paneWidth` is below the compact tier. */
export function isContentCompact(paneWidth: number): boolean {
  return paneWidth < CONTENT_COMPACT_WIDTH;
}

/** Project sidebar width to render: the user's dragged width normally, its
 *  minimum on the narrow tier. Never writes the dragged value back. */
export function projectSidebarWidth(dragged: number, viewport: number): number {
  return isNarrow(viewport) ? PROJECT_SIDEBAR_MIN : dragged;
}

/** Width for a side panel rendered as an overlay drawer — never wider than the
 *  box it overlays, less DRAWER_PEEK of the content behind it.
 *
 *  `available` is the width of the drawer's *containing block*, not the
 *  viewport: a drawer is positioned against its offset parent, so every in-flow
 *  column to that parent's left (the nav rail always; the project sidebar too,
 *  while it is still in flow at narrow) is already excluded. Passing the raw
 *  viewport here over-counts by exactly those columns and yields a drawer wider
 *  than the box it sits in — which clips it and eats the peek strip. Call sites
 *  measure (`useElementWidth`) or subtract the fixed rail themselves. */
export function drawerWidth(preferred: number, available: number): number {
  return Math.min(preferred, Math.max(0, available - DRAWER_PEEK));
}

/** Per-side floor for the agent split: SPLIT_PANE_MIN normally, half the row
 *  (less the handle) when the row cannot hold two SPLIT_PANE_MIN panes. */
export function splitPaneFloor(rowWidth: number): number {
  return Math.min(SPLIT_PANE_MIN, Math.floor((rowWidth - SPLIT_HANDLE_WIDTH) / 2));
}
