import { create } from "zustand";

/** Repo path whose settings dialog is open, or null. A module store so it can be
 *  opened from anywhere (project right-click menu, in-workstream scripts dock)
 *  while Root owns the single dialog instance. */
type ProjectSettingsState = { path: string | null };

export const useProjectSettingsStore = create<ProjectSettingsState>(() => ({ path: null }));

export const openProjectSettings = (path: string): void =>
  useProjectSettingsStore.setState({ path });
export const closeProjectSettings = (): void => useProjectSettingsStore.setState({ path: null });
