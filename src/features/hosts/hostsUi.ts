import { create } from "zustand";

type HostsUiState = { open: boolean; menu: boolean; update: boolean };

export const useHostsUiStore = create<HostsUiState>(() => ({ open: false, menu: false, update: false }));

export const openHostsUi = (): void => useHostsUiStore.setState({ open: true, menu: false });
export const closeHostsUi = (): void => useHostsUiStore.setState({ open: false });

export const openServerMenu = (): void => useHostsUiStore.setState({ menu: true });
export const closeServerMenu = (): void => useHostsUiStore.setState({ menu: false });
export const toggleServerMenu = (): void => useHostsUiStore.setState((s) => ({ menu: !s.menu }));

export const openServerUpdate = (): void => useHostsUiStore.setState({ update: true, menu: false });
export const closeServerUpdate = (): void => useHostsUiStore.setState({ update: false });
