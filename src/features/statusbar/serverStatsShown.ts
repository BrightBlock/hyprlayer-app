import { create } from "zustand";
import { persist } from "zustand/middleware";

type ServerStatsShown = { enabled: boolean };

// Machine-local: a remote server's settings are shared by every desktop that connects to it.
export const useServerStatsShown = create<ServerStatsShown>()(
  persist((): ServerStatsShown => ({ enabled: false }), { name: "hyprlayer:status-bar-server-stats", version: 1 }),
);

export function setServerStatsShown(enabled: boolean): void {
  useServerStatsShown.setState({ enabled });
}
