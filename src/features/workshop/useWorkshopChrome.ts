import { useMemo } from "react";

import type { WorkshopChrome } from "@/components/workshop";
import { visibleAgents, type AgentScope } from "@/features/agents/agents";
import { BUILTIN_PROMPTS, customPrompts } from "@/features/prompts/prompts";
import type { Project } from "@/lib/bindings";
import { useAgentFiles, useBootstrap, useSkillFiles } from "@/lib/queries";

import type { WorkshopSection } from "./workshopSearch";

export function useWorkshopChrome({
  section,
  onSection,
  project,
  onProject,
  projects,
}: {
  section: WorkshopSection;
  onSection: (section: WorkshopSection) => void;
  project: string | null;
  onProject: (path: string | null) => void;
  projects: Project[];
}): Omit<WorkshopChrome, "footer"> {
  const files = useAgentFiles(project).data;
  const skills = useSkillFiles(project).data;
  const settings = useBootstrap().data?.settings;
  const scope: AgentScope = project ? "project" : "global";

  const counts = useMemo(
    () =>
      files
        ? {
            agents: visibleAgents(files, { tab: "yours", scope, search: "" }).length,
            system: visibleAgents(files, { tab: "system", scope, search: "" }).length,
          }
        : null,
    [files, scope],
  );

  return {
    sections: [
      { key: "agents", label: "Your agents", icon: "layers", count: counts?.agents ?? null },
      { key: "system", label: "System agents", icon: "plugin", count: counts?.system ?? null },
      { key: "skills", label: "Skills", icon: "skill", count: skills?.length ?? null },
      {
        key: "prompts",
        label: "Prompts",
        icon: "prompt",
        count: settings ? BUILTIN_PROMPTS.length + customPrompts(settings).length : null,
      },
    ],
    active: section,
    onSection: (key) => onSection(key as WorkshopSection),
    // Prompts are global; the project param stays in the URL for the other sections.
    scope: section === "prompts" ? null : { projects, value: project, onChange: onProject },
  };
}
