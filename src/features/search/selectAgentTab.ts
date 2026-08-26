import { updateWorkstreamTabs } from "@/features/session/workstreamTabs";

export function selectAgentTab(workstreamId: string, sessionId: string): void {
  updateWorkstreamTabs((all) => {
    const ws = all[workstreamId];
    const shell = ws?.shells.find((s) => s.sessionId === sessionId);
    if (!ws || !shell) return all;
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
