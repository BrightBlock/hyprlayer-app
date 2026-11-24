import { create } from "zustand";
import { persist } from "zustand/middleware";

import { clamp } from "./resize";

const DIFF_SIDEBAR_WIDTH = 420;

export const DIFF_SIDEBAR_MIN = 380;
export const DIFF_SIDEBAR_MAX = 700;

const SIDE_PANEL_WIDTH = 288;
export const SIDE_PANEL_MIN = 220;

const DOC_RAIL_WIDTH = 300;
export const DOC_RAIL_MIN = 220;
export const DOC_RAIL_MAX = 420;

const WORKSHOP_LIST_WIDTH = 330;
export const WORKSHOP_LIST_MIN = 260;
export const WORKSHOP_LIST_MAX = 480;

export const LIBRARY_NAV_WIDTH = 208;
const LIBRARY_LIST_WIDTH = 344;
export const LIBRARY_LIST_MIN = 280;
export const LIBRARY_LIST_MAX = 480;

const LEAD_DOCK_WIDTH = 300;
export const LEAD_DOCK_MIN = 240;
export const LEAD_DOCK_MAX = 560;
// What the expanded dock's Lead panel leaves for the Lead's chat.
export const LEAD_DOCK_KEEP = 360;

function storedSize(value: unknown, fallback: number, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value) ? clamp(value, min, max) : fallback;
}

export type AsideView = "changes" | "plan" | "goal";

export type UiStore = {
  diffHidden: boolean;
  toggleDiff: () => void;
  diffWidth: number;
  setDiffWidth: (width: number) => void;
  tasksCollapsed: boolean;
  toggleTasks: () => void;
  leadDockExpanded: string[];
  setLeadDockExpanded: (projectPath: string, expanded: boolean) => void;
  // Persisted raw; readers resolve it against what the focused session offers.
  asideView: AsideView;
  // True once the person picks an aside view this run; auto-open never overrides that choice.
  asideViewChosen: boolean;
  setAsideView: (view: AsideView) => void;
  docsCollapsed: boolean;
  toggleDocs: () => void;
  libraryListWidth: number;
  setLibraryListWidth: (width: number) => void;
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
  skillDetailWide: boolean;
  setSkillDetailWide: (wide: boolean) => void;
  narrowDrawer: "project" | "diff" | "workshop" | "library" | null;
  setNarrowDrawer: (panel: "project" | "diff" | "workshop" | "library" | null) => void;
  // Remote folder picker; a display preference of the person, not the server,
  // which is why it sits here and not in a host-scoped store.
  showDotFiles: boolean;
  setShowDotFiles: (show: boolean) => void;
  docRailOpen: boolean;
  toggleDocRail: () => void;
  docRailWidth: number;
  setDocRailWidth: (width: number) => void;
  leadDockWidth: number;
  setLeadDockWidth: (width: number) => void;
  leadPanelHidden: boolean;
  toggleLeadPanel: () => void;
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
      leadDockExpanded: [],
      setLeadDockExpanded: (projectPath, expanded) =>
        set((state) => {
          const others = state.leadDockExpanded.filter((p) => p !== projectPath);
          return { leadDockExpanded: expanded ? [...others, projectPath] : others };
        }),
      asideView: "changes",
      asideViewChosen: false,
      setAsideView: (view) => set({ asideView: view, asideViewChosen: true }),
      docsCollapsed: false,
      toggleDocs: () => set((state) => ({ docsCollapsed: !state.docsCollapsed })),
      libraryListWidth: LIBRARY_LIST_WIDTH,
      setLibraryListWidth: (width) => set({ libraryListWidth: width }),
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
      docWide: true,
      setDocWide: (wide) => set({ docWide: wide }),
      skillDetailWide: true,
      setSkillDetailWide: (wide) => set({ skillDetailWide: wide }),
      narrowDrawer: null,
      setNarrowDrawer: (panel) => set({ narrowDrawer: panel }),
      showDotFiles: false,
      setShowDotFiles: (show) => set({ showDotFiles: show }),
      docRailOpen: true,
      toggleDocRail: () => set((state) => ({ docRailOpen: !state.docRailOpen })),
      docRailWidth: DOC_RAIL_WIDTH,
      setDocRailWidth: (width) => set({ docRailWidth: width }),
      leadDockWidth: LEAD_DOCK_WIDTH,
      setLeadDockWidth: (width) =>
        set({ leadDockWidth: storedSize(width, LEAD_DOCK_WIDTH, LEAD_DOCK_MIN, LEAD_DOCK_MAX) }),
      leadPanelHidden: false,
      toggleLeadPanel: () => set((state) => ({ leadPanelHidden: !state.leadPanelHidden })),
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
          leadDockExpanded: Array.isArray(p.leadDockExpanded) ? p.leadDockExpanded : [],
          leadDockWidth: storedSize(p.leadDockWidth, LEAD_DOCK_WIDTH, LEAD_DOCK_MIN, LEAD_DOCK_MAX),
          leadPanelHidden: p.leadPanelHidden === true,
        };
      },
      partialize: (s) => ({
        diffHidden: s.diffHidden,
        tasksCollapsed: s.tasksCollapsed,
        leadDockExpanded: s.leadDockExpanded,
        asideView: s.asideView,
        docsCollapsed: s.docsCollapsed,
        libraryCollapsed: s.libraryCollapsed,
        workshopNavCollapsed: s.workshopNavCollapsed,
        projectSidebarCollapsed: s.projectSidebarCollapsed,
        docWide: s.docWide,
        skillDetailWide: s.skillDetailWide,
        workshopListWidth: s.workshopListWidth,
        libraryListWidth: s.libraryListWidth,
        showDotFiles: s.showDotFiles,
        docRailOpen: s.docRailOpen,
        docRailWidth: s.docRailWidth,
        leadDockWidth: s.leadDockWidth,
        leadPanelHidden: s.leadPanelHidden,
        docRailContentsCollapsed: s.docRailContentsCollapsed,
        docRailPropertiesCollapsed: s.docRailPropertiesCollapsed,
      }),
    },
  ),
);
