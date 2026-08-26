import { useNavigate, useSearch } from "@tanstack/react-router";

import { PageError, PageSpinner } from "@/components/ui";
import { AgentLibrary } from "@/features/agents/AgentLibrary";
import { editorTabs, type AgentSection, type EditorTab } from "@/features/agents/agents";
import { useSignedIn } from "@/lib/account";
import { useBootstrap } from "@/lib/queries";

const AGENT_SECTIONS: readonly AgentSection[] = ["agents", "system", "skills"];

export function AgentsPage() {
  const search = useSearch({ from: "/agents" });
  const navigate = useNavigate({ from: "/agents" });
  const bootstrap = useBootstrap();
  const tabs = editorTabs(useSignedIn());

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const projects = bootstrap.data!.projects;
  const tab = tabs.some((t) => t.key === search.tab) ? (search.tab as EditorTab) : "setup";
  const section: AgentSection = AGENT_SECTIONS.includes(search.section as AgentSection)
    ? (search.section as AgentSection)
    : "agents";
  const selectedProject = projects.find((p) => p.path === search.project)?.path ?? null;

  return (
    <AgentLibrary
      projects={projects}
      selectedProject={selectedProject}
      onSelectProject={(path) =>
        void navigate({
          search: (prev) => ({
            ...prev,
            project: path ?? undefined,
            agent: undefined,
            tab: undefined,
          }),
          replace: true,
        })
      }
      selectedPath={search.agent ?? null}
      onSelect={(path) =>
        void navigate({
          search: (prev) => ({ ...prev, agent: path ?? undefined, tab: path ? prev.tab : undefined }),
          replace: true,
        })
      }
      editorTab={tab}
      onEditorTab={(next) =>
        void navigate({
          search: (prev) => ({ ...prev, tab: next === "setup" ? undefined : next }),
          replace: true,
        })
      }
      section={section}
      onSection={(next) =>
        void navigate({
          search: (prev) => ({
            ...prev,
            section: next === "agents" ? undefined : next,
            agent: undefined,
            tab: undefined,
          }),
          replace: true,
        })
      }
    />
  );
}
