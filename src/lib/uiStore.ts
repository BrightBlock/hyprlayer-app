import { create } from "zustand";
import { persist } from "zustand/middleware";

/**
 * Default width (px) of the right-hand diff sidebar. Matched to the left
 * project sidebar's default width so the two flank the workspace symmetrically.
 */
export const DIFF_SIDEBAR_WIDTH = 360;

/** Drag-resize bounds (px) for the right-hand diff sidebar. */
export const DIFF_SIDEBAR_MIN = 280;
export const DIFF_SIDEBAR_MAX = 640;

/** Default + drag-resize bounds (px) for the Library's list panel. Default is
 *  the mock's w-72; the bounds keep the switcher/tabs readable at the low end
 *  and stop it crowding the document at the high end. */
export const LIBRARY_SIDEBAR_WIDTH = 288;
export const LIBRARY_SIDEBAR_MIN = 220;
export const LIBRARY_SIDEBAR_MAX = 480;

/** Default + drag-resize bounds (px) for the prompt library's list panel. The
 *  default preserves the previous fixed `w-64`; the bounds keep the header
 *  actions + chord hints readable at the low end and stop it crowding the
 *  editor at the high end. */
export const PROMPTS_SIDEBAR_WIDTH = 256;
export const PROMPTS_SIDEBAR_MIN = 200;
export const PROMPTS_SIDEBAR_MAX = 420;

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
  /** Whether the Tasks section inside the right diff sidebar — the agent's live
   *  plan/to-do list, docked under the Git section — is collapsed to its header.
   *  A sidebar-level layout preference (not per-session, so it stays put as you
   *  switch agent tabs) persisted like the other collapse flags. */
  tasksCollapsed: boolean;
  toggleTasks: () => void;
  /** Live width (px) of the Library's list panel. Lives here (not in the route
   *  component) so a drag survives leaving and re-opening the Library — the
   *  route unmounts, but this store doesn't. */
  libraryWidth: number;
  setLibraryWidth: (width: number) => void;
  /** Live width (px) of the prompt library's list panel. Session-only like
   *  `libraryWidth` — survives navigation via the store but resets on restart
   *  (excluded from `partialize`, so a drag never writes to localStorage). */
  promptsWidth: number;
  setPromptsWidth: (width: number) => void;
  /** Whether the Library's list panel is collapsed. Mirrors the project panel's
   *  collapse so the header sidebar toggle works the same on the Library route;
   *  here (not the route) so it survives leaving and re-opening the Library. */
  libraryCollapsed: boolean;
  toggleLibrary: () => void;
  /** Whether the left project sidebar is collapsed. Persisted (like the other
   *  collapse states) so the app reopens with the same layout. Lives here rather
   *  than as Root-local state so it survives an app restart. */
  projectSidebarCollapsed: boolean;
  setProjectSidebarCollapsed: (collapsed: boolean) => void;
  toggleProjectSidebar: () => void;
  /** Whether the Library's document detail uses the full width ("wide") instead
   *  of a centered readable column ("narrow"). A per-user reading/editing
   *  preference, applied to both preview and the editor; chosen via the header's
   *  Narrow|Wide segmented control. */
  docWide: boolean;
  setDocWide: (wide: boolean) => void;
};

export const useUiStore = create<UiStore>()(
  persist(
    (set) => ({
      diffHidden: false,
      toggleDiff: () => set((state) => ({ diffHidden: !state.diffHidden })),
      diffWidth: DIFF_SIDEBAR_WIDTH,
      setDiffWidth: (width) => set({ diffWidth: width }),
      tasksCollapsed: false,
      toggleTasks: () => set((state) => ({ tasksCollapsed: !state.tasksCollapsed })),
      libraryWidth: LIBRARY_SIDEBAR_WIDTH,
      setLibraryWidth: (width) => set({ libraryWidth: width }),
      promptsWidth: PROMPTS_SIDEBAR_WIDTH,
      setPromptsWidth: (width) => set({ promptsWidth: width }),
      libraryCollapsed: false,
      toggleLibrary: () => set((state) => ({ libraryCollapsed: !state.libraryCollapsed })),
      projectSidebarCollapsed: false,
      setProjectSidebarCollapsed: (collapsed) => set({ projectSidebarCollapsed: collapsed }),
      toggleProjectSidebar: () =>
        set((state) => ({ projectSidebarCollapsed: !state.projectSidebarCollapsed })),
      docWide: false,
      setDocWide: (wide) => set({ docWide: wide }),
    }),
    {
      name: "hyprlayer:ui",
      version: 1,
      // Persist only the layout/preference flags — NOT the live drag widths,
      // which would otherwise write to localStorage on every resize pointermove.
      partialize: (s) => ({
        diffHidden: s.diffHidden,
        tasksCollapsed: s.tasksCollapsed,
        libraryCollapsed: s.libraryCollapsed,
        projectSidebarCollapsed: s.projectSidebarCollapsed,
        docWide: s.docWide,
      }),
    },
  ),
);
