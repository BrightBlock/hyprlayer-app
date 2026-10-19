import {
  focusShell,
  openSessionTab,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
} from "@/features/session/workstreamTabs";
import { workstreamSessionsQuery } from "@/lib/queries";
import { queryClient } from "@/lib/queryClient";

export async function selectAgentTab(
  workstreamId: string,
  sessionId: string,
  opts?: { frozen?: boolean },
): Promise<void> {
  const local = useWorkstreamTabsStore
    .getState()
    .byWs[workstreamId]?.shells.find((s) => s.sessionId === sessionId);
  if (local) {
    updateWorkstreamTabs((all) => {
      const ws = all[workstreamId];
      return ws ? { ...all, [workstreamId]: focusShell(ws, local.id) } : all;
    });
    return;
  }

  const sessions = await queryClient.fetchQuery(workstreamSessionsQuery(workstreamId)).catch(() => []);
  const session = sessions.find((s) => s.id === sessionId);
  if (session) openSessionTab(workstreamId, session, opts);
}
