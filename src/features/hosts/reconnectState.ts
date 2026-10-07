import { create } from "zustand";

import type { AppError } from "@/lib/bindings";

/** Why the loop stopped dialling: binaries that cannot talk, or a sign-in the user cancelled. */
export type ReconnectHalt = "protocol" | "signIn";

export type ReconnectSnapshot = {
  lostAt: number | null;
  attempting: boolean;
  halted: ReconnectHalt | null;
  /** The last attempt met a changed host key, which only the user can clear. */
  keyChanged: AppError | null;
};

// Written only by ./reconnect; the status bar reads it beside the connection status.
export const useReconnectStore = create<ReconnectSnapshot>(() => ({
  lostAt: null,
  attempting: false,
  halted: null,
  keyChanged: null,
}));
