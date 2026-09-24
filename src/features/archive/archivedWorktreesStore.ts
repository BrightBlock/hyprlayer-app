import { create } from "zustand";

// The header button and the Dashboard's "View all" both drive this one flag.
export const useArchivedWorktrees = create<{ open: boolean }>(() => ({ open: false }));

export const openArchivedWorktrees = (): void => useArchivedWorktrees.setState({ open: true });
export const closeArchivedWorktrees = (): void => useArchivedWorktrees.setState({ open: false });
export const toggleArchivedWorktrees = (): void =>
  useArchivedWorktrees.setState((s) => ({ open: !s.open }));
