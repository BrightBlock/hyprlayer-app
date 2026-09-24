import { create } from "zustand";

// The gear, ⌘, and the macOS menu item all drive this one flag.
export const useSettingsPopover = create<{ open: boolean }>(() => ({ open: false }));

export const openSettingsPopover = (): void => useSettingsPopover.setState({ open: true });
export const closeSettingsPopover = (): void => useSettingsPopover.setState({ open: false });
export const toggleSettingsPopover = (): void =>
  useSettingsPopover.setState((s) => ({ open: !s.open }));
