import { useNavigate, useSearch } from "@tanstack/react-router";

import { PageError, PageSpinner } from "@/components/ui";
import { AgentLibrary } from "@/features/agents/AgentLibrary";
import { EDITOR_TABS, type EditorTab } from "@/features/agents/agents";
import { useLibraryScope } from "@/features/library/libraryScope";
import { useBootstrap } from "@/lib/queries";

/**
 * The Agents route (`/agents`), opened from the nav rail. The open agent's file
 * path and its editor tab live in the URL so a view is restorable; a bare
 * `/agents` is the list.
 */
export function AgentsPage() {
  const search = useSearch({ from: "/agents" });
  const navigate = useNavigate({ from: "/agents" });
  const bootstrap = useBootstrap();
  // Agents is a global surface, so "this project" means the last one selected —
  // the same signal the Library seeds its scope from.
  const lastProject = useLibraryScope((s) => s.lastProject);

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const projects = bootstrap.data!.projects;
  const projectPath =
    projects.find((p) => p.thoughtsProject === lastProject)?.path ?? projects[0]?.path ?? null;
  const tab = EDITOR_TABS.some((t) => t.key === search.tab) ? (search.tab as EditorTab) : "setup";

  return (
    <AgentLibrary
      projectPath={projectPath}
      selectedPath={search.agent ?? null}
      // Both updates read the LIVE search, not this render's: opening an agent
      // also sets its tab, and two navigates in one tick would otherwise have
      // the second clobber the first with a stale value.
      onSelect={(path) =>
        void navigate({
          search: (prev) => ({ agent: path ?? undefined, tab: path ? prev.tab : undefined }),
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
    />
  );
}
