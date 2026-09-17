import { focusShell, updateWorkstreamTabs } from "@/features/session/workstreamTabs";

export function selectAgentTab(workstreamId: string, sessionId: string): void {
  updateWorkstreamTabs((all) => {
    const ws = all[workstreamId];
    const shell = ws?.shells.find((s) => s.sessionId === sessionId);
    if (!ws || !shell) return all;
    return { ...all, [workstreamId]: focusShell(ws, shell.id) };
  });
}
