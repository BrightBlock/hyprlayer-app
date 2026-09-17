import { create } from "zustand";

type HostsUiState = { open: boolean };

export const useHostsUiStore = create<HostsUiState>(() => ({ open: false }));

export const openHostsUi = (): void => useHostsUiStore.setState({ open: true });
export const closeHostsUi = (): void => useHostsUiStore.setState({ open: false });
