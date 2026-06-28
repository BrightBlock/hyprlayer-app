import { create } from "zustand";

/** The project (by repo path) whose settings dialog is open, or null. Held in a
 *  module store so it can be opened from anywhere — the project right-click menu
 *  AND the in-workstream scripts dock's "Edit scripts ↗" — while Root owns the
 *  single dialog instance. */
type ProjectSettingsState = { path: string | null };

export const useProjectSettingsStore = create<ProjectSettingsState>(() => ({ path: null }));

export const openProjectSettings = (path: string): void =>
  useProjectSettingsStore.setState({ path });
export const closeProjectSettings = (): void => useProjectSettingsStore.setState({ path: null });
