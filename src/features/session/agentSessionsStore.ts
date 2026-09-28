import { create } from "zustand";

export type SessionsView = "current" | "archived";

// The status-bar button and ⌘⇧A drive this; Root closes it when a palette opens.
export const useAgentSessions = create<{ open: boolean; view: SessionsView; query: string }>(() => ({
  open: false,
  view: "current",
  query: "",
}));

export const closeAgentSessions = (): void => useAgentSessions.setState({ open: false, query: "" });

export const toggleAgentSessions = (): void =>
  useAgentSessions.setState((s) => (s.open ? { open: false, query: "" } : { open: true, view: "current" }));

// ⌘⇧A: straight to the archive, or closes it when it is already showing.
export const toggleArchivedSessions = (): void =>
  useAgentSessions.setState((s) =>
    s.open && s.view === "archived" ? { open: false, query: "" } : { open: true, view: "archived" },
  );

export const setSessionsView = (view: SessionsView): void => useAgentSessions.setState({ view });

export const setSessionsQuery = (query: string): void => useAgentSessions.setState({ query });
