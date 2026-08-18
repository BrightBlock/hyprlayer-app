import { create } from "zustand";
import { persist } from "zustand/middleware";

/** Default width (px) of the right-hand diff sidebar. */
export const DIFF_SIDEBAR_WIDTH = 420;

/** Drag-resize bounds (px) for the right-hand diff sidebar. */
export const DIFF_SIDEBAR_MIN = 400;
export const DIFF_SIDEBAR_MAX = 700;

/** Default + drag-resize bounds (px) for the panels that replace the project
 *  tree in the shell's second column, Library, Prompts and Settings. One set of
 *  numbers for all three so the column doesn't jump width as you navigate
 *  between those routes. Default is the mock's w-72; the bounds keep each
 *  panel's rows readable at the low end and stop them crowding the content
 *  column at the high end. (Settings is fixed at the default, its labels are
 *  short and none of them wrap, so there's nothing to drag for.) */
export const SIDE_PANEL_WIDTH = 288;
export const SIDE_PANEL_MIN = 220;
export const SIDE_PANEL_MAX = 480;

type UiStore = {
  /**
   * Whether the right-hand diff sidebar is hidden. Shared state so the top-bar
   * toggle (in the Root header) and the sidebar itself (in ProjectPage) stay in
   * sync, when hidden, ProjectPage drops the aside entirely rather than showing
   * a collapsed rail.
   */
  diffHidden: boolean;
  toggleDiff: () => void;
  /**
   * Live width (px) of the right-hand diff sidebar. The single source of truth
   * for the sidebar itself, so the Root header's toggle and ProjectPage's
   * resize handle stay in sync without either owning the value.
   */
  diffWidth: number;
  setDiffWidth: (width: number) => void;
  /** Whether the Tasks section inside the right diff sidebar, the agent's live
   *  plan/to-do list, docked under the Git section, is collapsed to its header.
   *  A sidebar-level layout preference (not per-session, so it stays put as you
   *  switch agent tabs) persisted like the other collapse flags. */
  tasksCollapsed: boolean;
  toggleTasks: () => void;
  /** Live width (px) of the Library's list panel. Lives here (not in the route
   *  component) so a drag survives leaving and re-opening the Library, the
   *  route unmounts, but this store doesn't. */
  libraryWidth: number;
  setLibraryWidth: (width: number) => void;
  /** Live width (px) of the prompt library's list panel. Session-only like
   *  `libraryWidth`, survives navigation via the store but resets on restart
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
  /** Which side panel is open as an overlay drawer on the narrow layout tier
   *  (see lib/layout). One at a time, a ~700px window has room for exactly
   *  one. Deliberately NOT persisted and never auto-set: the narrow tier
   *  derives its behaviour at render and leaves `diffHidden` /
   *  `projectSidebarCollapsed` (the user's real preferences, restored the
   *  moment the window widens) untouched. Cleared by Root when the tier that
   *  owns it is left. */
  narrowDrawer: "project" | "diff" | null;
  setNarrowDrawer: (panel: "project" | "diff" | null) => void;
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
      libraryWidth: SIDE_PANEL_WIDTH,
      setLibraryWidth: (width) => set({ libraryWidth: width }),
      promptsWidth: SIDE_PANEL_WIDTH,
      setPromptsWidth: (width) => set({ promptsWidth: width }),
      libraryCollapsed: false,
      toggleLibrary: () => set((state) => ({ libraryCollapsed: !state.libraryCollapsed })),
      projectSidebarCollapsed: false,
      setProjectSidebarCollapsed: (collapsed) => set({ projectSidebarCollapsed: collapsed }),
      toggleProjectSidebar: () =>
        set((state) => ({ projectSidebarCollapsed: !state.projectSidebarCollapsed })),
      docWide: false,
      setDocWide: (wide) => set({ docWide: wide }),
      narrowDrawer: null,
      setNarrowDrawer: (panel) => set({ narrowDrawer: panel }),
    }),
    {
      name: "hyprlayer:ui",
      version: 1,
      // Persist only the layout/preference flags, NOT the live drag widths,
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
