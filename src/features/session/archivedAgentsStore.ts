import { create } from "zustand";

// The status-bar button and ⌘⇧A both drive this one flag.
export const useArchivedAgents = create<{ open: boolean }>(() => ({ open: false }));

export const closeArchivedAgents = (): void => useArchivedAgents.setState({ open: false });
export const toggleArchivedAgents = (): void => useArchivedAgents.setState((s) => ({ open: !s.open }));
