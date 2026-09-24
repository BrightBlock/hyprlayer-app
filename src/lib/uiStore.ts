import { create } from "zustand";
import { persist } from "zustand/middleware";

export const DIFF_SIDEBAR_WIDTH = 420;

export const DIFF_SIDEBAR_MIN = 380;
export const DIFF_SIDEBAR_MAX = 700;

export const SIDE_PANEL_WIDTH = 288;
export const SIDE_PANEL_MIN = 220;
export const SIDE_PANEL_MAX = 480;

type UiStore = {
  diffHidden: boolean;
  toggleDiff: () => void;
  diffWidth: number;
  setDiffWidth: (width: number) => void;
  tasksCollapsed: boolean;
  toggleTasks: () => void;
  asideView: "changes" | "plan";
  setAsideView: (view: "changes" | "plan") => void;
  docsCollapsed: boolean;
  toggleDocs: () => void;
  libraryWidth: number;
  setLibraryWidth: (width: number) => void;
  promptsWidth: number;
  setPromptsWidth: (width: number) => void;
  agentsWidth: number;
  setAgentsWidth: (width: number) => void;
  libraryCollapsed: boolean;
  toggleLibrary: () => void;
  agentsCollapsed: boolean;
  toggleAgents: () => void;
  projectSidebarCollapsed: boolean;
  setProjectSidebarCollapsed: (collapsed: boolean) => void;
  toggleProjectSidebar: () => void;
  docWide: boolean;
  setDocWide: (wide: boolean) => void;
  narrowDrawer: "project" | "diff" | null;
  setNarrowDrawer: (panel: "project" | "diff" | null) => void;
  // Remote folder picker; a display preference of the person, not the server,
  // which is why it sits here and not in a host-scoped store.
  showDotFiles: boolean;
  setShowDotFiles: (show: boolean) => void;
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
      setAsideView: (view) => set({ asideView: view }),
      docsCollapsed: false,
      toggleDocs: () => set((state) => ({ docsCollapsed: !state.docsCollapsed })),
      libraryWidth: SIDE_PANEL_WIDTH,
      setLibraryWidth: (width) => set({ libraryWidth: width }),
      promptsWidth: SIDE_PANEL_WIDTH,
      setPromptsWidth: (width) => set({ promptsWidth: width }),
      agentsWidth: SIDE_PANEL_WIDTH,
      setAgentsWidth: (width) => set({ agentsWidth: width }),
      libraryCollapsed: false,
      toggleLibrary: () => set((state) => ({ libraryCollapsed: !state.libraryCollapsed })),
      agentsCollapsed: false,
      toggleAgents: () => set((state) => ({ agentsCollapsed: !state.agentsCollapsed })),
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
    }),
    {
      name: "hyprlayer:ui",
      version: 1,
      partialize: (s) => ({
        diffHidden: s.diffHidden,
        tasksCollapsed: s.tasksCollapsed,
        asideView: s.asideView,
        docsCollapsed: s.docsCollapsed,
        libraryCollapsed: s.libraryCollapsed,
        agentsCollapsed: s.agentsCollapsed,
        projectSidebarCollapsed: s.projectSidebarCollapsed,
        docWide: s.docWide,
        agentsWidth: s.agentsWidth,
        showDotFiles: s.showDotFiles,
      }),
    },
  ),
);
