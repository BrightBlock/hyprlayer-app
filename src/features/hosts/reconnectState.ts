import { create } from "zustand";

export type ReconnectSnapshot = { lostAt: number | null; attempting: boolean; halted: boolean };

// Written only by ./reconnect; the status bar reads it beside the connection status.
export const useReconnectStore = create<ReconnectSnapshot>(() => ({
  lostAt: null,
  attempting: false,
  halted: false,
}));
