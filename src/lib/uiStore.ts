import { create } from "zustand";

type UiStore = {
  /**
   * Whether the right-hand diff sidebar is hidden. Shared state so the top-bar
   * toggle (in the Root header) and the sidebar itself (in ProjectPage) stay in
   * sync — when hidden, ProjectPage drops the aside entirely rather than showing
   * a collapsed rail.
   */
  diffHidden: boolean;
  toggleDiff: () => void;
};

export const useUiStore = create<UiStore>((set) => ({
  diffHidden: false,
  toggleDiff: () => set((state) => ({ diffHidden: !state.diffHidden })),
}));
