import { create } from "zustand";

// The status-bar button drives this flag; Root closes it when a palette opens.
export const useAgentSessions = create<{ open: boolean }>(() => ({ open: false }));

export const closeAgentSessions = (): void => useAgentSessions.setState({ open: false });
export const toggleAgentSessions = (): void => useAgentSessions.setState((s) => ({ open: !s.open }));
