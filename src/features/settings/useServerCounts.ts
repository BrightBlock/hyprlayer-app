import { serverRows, serverSummary } from "@/lib/domain/hosts";
import { useConnectionStatus, useHostProbes, useHosts } from "@/lib/queries";

import type { ServerCounts } from "./settingsMeta";

/** Rows the Servers page would show, counted; offline covers the live link and this session's probes. */
export function useServerCounts(): ServerCounts {
  const file = useHosts().data;
  const connection = useConnectionStatus().data;
  const probes = useHostProbes();
  return serverSummary(serverRows(file, connection, probes));
}
