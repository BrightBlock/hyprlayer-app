import { useNavigate, useSearch } from "@tanstack/react-router";

import { PageError, PageSpinner } from "@/components/ui";
import { AgentLibrary } from "@/features/agents/AgentLibrary";
import { editorTabs, type AgentSection, type EditorTab } from "@/features/agents/agents";
import { useSignedIn } from "@/lib/account";
import { useBootstrap } from "@/lib/queries";

const AGENT_SECTIONS: readonly AgentSection[] = ["agents", "system", "skills"];

/**
 * The Agents route (`/agents`), opened from the nav rail. The open agent's file
 * path and its editor tab live in the URL so a view is restorable; a bare
 * `/agents` is the list. `?section=` names the sidebar's root nav choice — an
 * unknown or absent value falls back to `agents`. `?project=` scopes the list
 * to one project's `.claude/agents`; absent means the global library, which is
 * what a bare `/agents` has always opened on.
 */
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
  // Resolved against the live project list, so a `?project=` naming a project
  // that has since been removed falls back to the global library rather than
  // scoping to a path nothing can be read from.
  const selectedProject = projects.find((p) => p.path === search.project)?.path ?? null;

  return (
    <AgentLibrary
      projects={projects}
      selectedProject={selectedProject}
      // `null` clears the param back to the global library. The open agent is
      // dropped with it: it belongs to the scope being left, so keeping it
      // would leave the editor showing an agent absent from the list.
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
      // Every navigate below spreads `prev` first: `section` and `project` are
      // not part of what these handlers set, and without the spread each one
      // would clear them back to their defaults the moment it fired.
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
      // Section and agent are not independent (see AgentLibrary): any root nav
      // change is a "go to that section's list" — it always drops the open
      // agent, mirroring the sidebar's own back chevron. The project scope
      // survives, so switching to System agents and back keeps your project.
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
