import { create } from "zustand";

type ProjectSettingsState = { path: string | null };

export const useProjectSettingsStore = create<ProjectSettingsState>(() => ({ path: null }));

export const openProjectSettings = (path: string): void =>
  useProjectSettingsStore.setState({ path });
export const closeProjectSettings = (): void => useProjectSettingsStore.setState({ path: null });
