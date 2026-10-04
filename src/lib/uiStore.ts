import { create } from "zustand";
import { persist } from "zustand/middleware";

import { clamp } from "./resize";

export const DIFF_SIDEBAR_WIDTH = 420;

export const DIFF_SIDEBAR_MIN = 380;
export const DIFF_SIDEBAR_MAX = 700;

export const SIDE_PANEL_WIDTH = 288;
export const SIDE_PANEL_MIN = 220;
export const SIDE_PANEL_MAX = 480;

// Matches the left sidebar's default width (SIDE_PANEL_WIDTH) by design.
export const DOC_RAIL_WIDTH = SIDE_PANEL_WIDTH;
export const DOC_RAIL_MIN = 220;
export const DOC_RAIL_MAX = 420;

export const WORKSHOP_LIST_WIDTH = 330;
export const WORKSHOP_LIST_MIN = 260;
export const WORKSHOP_LIST_MAX = 480;

export type UiStore = {
  diffHidden: boolean;
  toggleDiff: () => void;
  diffWidth: number;
  setDiffWidth: (width: number) => void;
  tasksCollapsed: boolean;
  toggleTasks: () => void;
  asideView: "changes" | "plan";
  // True once the person picks an aside view this run; auto-open never overrides that choice.
  asideViewChosen: boolean;
  setAsideView: (view: "changes" | "plan") => void;
  docsCollapsed: boolean;
  toggleDocs: () => void;
  libraryWidth: number;
  setLibraryWidth: (width: number) => void;
  workshopListWidth: number;
  setWorkshopListWidth: (width: number) => void;
  libraryCollapsed: boolean;
  toggleLibrary: () => void;
  workshopNavCollapsed: boolean;
  toggleWorkshopNav: () => void;
  projectSidebarCollapsed: boolean;
  setProjectSidebarCollapsed: (collapsed: boolean) => void;
  toggleProjectSidebar: () => void;
  docWide: boolean;
  setDocWide: (wide: boolean) => void;
  narrowDrawer: "project" | "diff" | "workshop" | null;
  setNarrowDrawer: (panel: "project" | "diff" | "workshop" | null) => void;
  // Remote folder picker; a display preference of the person, not the server,
  // which is why it sits here and not in a host-scoped store.
  showDotFiles: boolean;
  setShowDotFiles: (show: boolean) => void;
  docRailOpen: boolean;
  toggleDocRail: () => void;
  docRailWidth: number;
  setDocRailWidth: (width: number) => void;
  docRailContentsCollapsed: boolean;
  toggleDocRailContents: () => void;
  docRailPropertiesCollapsed: boolean;
  toggleDocRailProperties: () => void;
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
      asideView: "changes",
      asideViewChosen: false,
      setAsideView: (view) => set({ asideView: view, asideViewChosen: true }),
      docsCollapsed: false,
      toggleDocs: () => set((state) => ({ docsCollapsed: !state.docsCollapsed })),
      libraryWidth: SIDE_PANEL_WIDTH,
      setLibraryWidth: (width) => set({ libraryWidth: width }),
      workshopListWidth: WORKSHOP_LIST_WIDTH,
      setWorkshopListWidth: (width) => set({ workshopListWidth: width }),
      libraryCollapsed: false,
      toggleLibrary: () => set((state) => ({ libraryCollapsed: !state.libraryCollapsed })),
      workshopNavCollapsed: false,
      toggleWorkshopNav: () =>
        set((state) => ({ workshopNavCollapsed: !state.workshopNavCollapsed })),
      projectSidebarCollapsed: false,
      setProjectSidebarCollapsed: (collapsed) => set({ projectSidebarCollapsed: collapsed }),
      toggleProjectSidebar: () =>
        set((state) => ({ projectSidebarCollapsed: !state.projectSidebarCollapsed })),
      docWide: false,
      setDocWide: (wide) => set({ docWide: wide }),
      narrowDrawer: null,
      setNarrowDrawer: (panel) => set({ narrowDrawer: panel }),
      showDotFiles: false,
      setShowDotFiles: (show) => set({ showDotFiles: show }),
      docRailOpen: true,
      toggleDocRail: () => set((state) => ({ docRailOpen: !state.docRailOpen })),
      docRailWidth: DOC_RAIL_WIDTH,
      setDocRailWidth: (width) => set({ docRailWidth: width }),
      docRailContentsCollapsed: false,
      toggleDocRailContents: () =>
        set((state) => ({ docRailContentsCollapsed: !state.docRailContentsCollapsed })),
      docRailPropertiesCollapsed: false,
      toggleDocRailProperties: () =>
        set((state) => ({ docRailPropertiesCollapsed: !state.docRailPropertiesCollapsed })),
    }),
    {
      name: "hyprlayer:ui",
      // A version bump would make an older build drop the whole blob; merge carries the pre-Workshop keys.
      version: 1,
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Partial<UiStore> & {
          agentsWidth?: unknown;
          agentsCollapsed?: unknown;
        };
        const { agentsWidth, agentsCollapsed, ...rest } = p;
        return {
          ...current,
          ...rest,
          workshopListWidth:
            typeof p.workshopListWidth === "number"
              ? p.workshopListWidth
              : typeof agentsWidth === "number" && agentsWidth !== SIDE_PANEL_WIDTH
                ? clamp(agentsWidth, WORKSHOP_LIST_MIN, WORKSHOP_LIST_MAX)
                : WORKSHOP_LIST_WIDTH,
          workshopNavCollapsed:
            typeof p.workshopNavCollapsed === "boolean"
              ? p.workshopNavCollapsed
              : agentsCollapsed === true,
        };
      },
      partialize: (s) => ({
        diffHidden: s.diffHidden,
        tasksCollapsed: s.tasksCollapsed,
        asideView: s.asideView,
        docsCollapsed: s.docsCollapsed,
        libraryCollapsed: s.libraryCollapsed,
        workshopNavCollapsed: s.workshopNavCollapsed,
        projectSidebarCollapsed: s.projectSidebarCollapsed,
        docWide: s.docWide,
        workshopListWidth: s.workshopListWidth,
        showDotFiles: s.showDotFiles,
        docRailOpen: s.docRailOpen,
        docRailWidth: s.docRailWidth,
        docRailContentsCollapsed: s.docRailContentsCollapsed,
        docRailPropertiesCollapsed: s.docRailPropertiesCollapsed,
      }),
    },
  ),
);
