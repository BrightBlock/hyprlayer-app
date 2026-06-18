import { create } from "zustand";

/**
 * Default width (px) of the right-hand diff sidebar. Matched to the left
 * project sidebar's default width so the two flank the workspace symmetrically.
 */
export const DIFF_SIDEBAR_WIDTH = 360;

/** Drag-resize bounds (px) for the right-hand diff sidebar. */
export const DIFF_SIDEBAR_MIN = 280;
export const DIFF_SIDEBAR_MAX = 640;

type UiStore = {
  /**
   * Whether the right-hand diff sidebar is hidden. Shared state so the top-bar
   * toggle (in the Root header) and the sidebar itself (in ProjectPage) stay in
   * sync — when hidden, ProjectPage drops the aside entirely rather than showing
   * a collapsed rail.
   */
  diffHidden: boolean;
  toggleDiff: () => void;
  /**
   * Live width (px) of the right-hand diff sidebar. The single source of truth
   * for both the sidebar itself and the Root header, which insets its
   * right-side controls by this amount so they align with the agent pane's
   * right margin (mirroring how the breadcrumb tracks the left sidebar). If the
   * sidebar ever becomes drag-resizable, updating this here makes the header
   * controls follow automatically.
   */
  diffWidth: number;
  setDiffWidth: (width: number) => void;
};

export const useUiStore = create<UiStore>((set) => ({
  diffHidden: false,
  toggleDiff: () => set((state) => ({ diffHidden: !state.diffHidden })),
  diffWidth: DIFF_SIDEBAR_WIDTH,
  setDiffWidth: (width) => set({ diffWidth: width }),
}));
