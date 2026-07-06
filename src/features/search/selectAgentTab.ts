import { updateWorkstreamTabs } from "@/features/session/workstreamTabs";

/**
 * After navigating to a workstream, best-effort select the tab bound to
 * `sessionId`. A no-op if the tab doesn't exist yet — opening the workstream
 * is enough to see the session.
 */
export function selectAgentTab(workstreamId: string, sessionId: string): void {
  updateWorkstreamTabs((all) => {
    const ws = all[workstreamId];
    const shell = ws?.shells.find((s) => s.sessionId === sessionId);
    if (!ws || !shell) return all; // tab not built yet; opening the workstream is enough
    const right = shell.panel === "right";
    return {
      ...all,
      [workstreamId]: {
        ...ws,
        ...(right
          ? { rightActiveId: shell.id, rightSplit: true }
          : { activeId: shell.id }),
        activePane: right ? "right" : "main",
      },
    };
  });
}
