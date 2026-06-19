import { create } from "zustand";

type SearchState = { open: boolean };

export const useSearchStore = create<SearchState>(() => ({ open: false }));

export const openSearch = (): void => useSearchStore.setState({ open: true });
export const closeSearch = (): void => useSearchStore.setState({ open: false });
export const toggleSearch = (): void => useSearchStore.setState((s) => ({ open: !s.open }));
