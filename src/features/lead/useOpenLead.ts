import { useNavigate } from "@tanstack/react-router";

import { useCommand } from "@/lib/keybindings/registry";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { setLeadDockExpanded, useProjectLead } from "./lead";

/** "Open Lead Engineer": from anywhere in the project, its board with the Lead's chat expanded. */
export function useOpenLeadCommand(projectPath: string): void {
  const navigate = useNavigate();
  const lead = useProjectLead(projectPath);
  const sessionId = lead?.view.sessionId ?? null;
  const workstreamId = lead?.view.workstreamId ?? null;
  useCommand(
    "lead.open",
    sessionId && workstreamId
      ? () => {
          void navigate({ to: "/projects/$projectPath", params: { projectPath }, search: {} });
          void selectAgentTab(workstreamId, sessionId);
          setLeadDockExpanded(projectPath, true);
        }
      : null,
  );
}
