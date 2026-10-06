import { useNavigate, useSearch } from "@tanstack/react-router";

import { PageError, PageSpinner } from "@/components/ui";
import { AgentLibrary } from "@/features/agents/AgentLibrary";
import { editorTabs, type EditorTab } from "@/features/agents/agents";
import { PromptLibrary } from "@/features/prompts/PromptLibrary";
import { promptFilter } from "@/features/prompts/prompts";
import { BundleFooter, BundleUpdateDialogs } from "@/features/workshop/BundleFooter";
import { SkillLibrary } from "@/features/workshop/SkillLibrary";
import { skillTab } from "@/features/workshop/skills";
import { useWorkshopChrome } from "@/features/workshop/useWorkshopChrome";
import { workshopSection } from "@/features/workshop/workshopSearch";
import { useSignedIn } from "@/lib/account";
import { useBootstrap } from "@/lib/queries";

export function WorkshopPage() {
  const search = useSearch({ from: "/workshop" });
  const navigate = useNavigate({ from: "/workshop" });
  const bootstrap = useBootstrap();
  const tabs = editorTabs(useSignedIn());

  const projects = bootstrap.data?.projects ?? [];
  const section = workshopSection(search.section);
  const project = projects.find((p) => p.path === search.project) ?? null;
  const projectPath = project?.path ?? null;

  const chrome = {
    ...useWorkshopChrome({
      section,
      onSection: (next) =>
        void navigate({
          search: (prev) => ({
            ...prev,
            section: next === "agents" ? undefined : next,
            agent: undefined,
            tab: undefined,
            folder: undefined,
            filter: undefined,
            view: undefined,
            skill: undefined,
            prompt: undefined,
          }),
          replace: true,
        }),
      project: projectPath,
      onProject: (path) =>
        void navigate({
          search: (prev) => ({
            ...prev,
            project: path ?? undefined,
            agent: undefined,
            tab: undefined,
            skill: undefined,
          }),
          replace: true,
        }),
      projects,
    }),
    footer: <BundleFooter />,
  };

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const tab = tabs.some((t) => t.key === search.tab) ? (search.tab as EditorTab) : "setup";

  return (
    <>
      {section === "skills" ? (
        <SkillLibrary
          chrome={chrome}
          projectPath={projectPath}
          projectName={project?.mappedName ?? null}
          selectedPath={search.skill ?? null}
          onSelect={(path) =>
            void navigate({
              search: (prev) => ({ ...prev, skill: path ?? undefined, tab: path ? prev.tab : undefined }),
              replace: true,
            })
          }
          filter={search.filter}
          onFilter={(filter) =>
            void navigate({
              search: (prev) => ({ ...prev, filter: filter === "all" ? undefined : filter, view: undefined }),
              replace: true,
            })
          }
          view={search.view}
          onView={(view) =>
            void navigate({
              search: (prev) => ({ ...prev, view: view ?? undefined, skill: undefined }),
              replace: true,
            })
          }
          tab={skillTab(search.tab)}
          onTab={(next) =>
            void navigate({
              search: (prev) => ({ ...prev, tab: next === "instructions" ? undefined : next }),
              replace: true,
            })
          }
          onOpenAgent={(path, target) =>
            void navigate({
              search: {
                section: target === "system" ? "system" : undefined,
                project: search.project,
                agent: path,
              },
            })
          }
        />
      ) : section === "prompts" ? (
        <PromptLibrary
          chrome={chrome}
          settings={bootstrap.data!.settings}
          selectedId={search.prompt ?? "commit"}
          onSelect={(id) =>
            void navigate({
              search: (prev) => ({ ...prev, prompt: id === "commit" ? undefined : id }),
              replace: true,
            })
          }
          filter={promptFilter(search.filter)}
          onFilter={(filter) =>
            void navigate({
              search: (prev) => ({ ...prev, filter: filter === "all" ? undefined : filter }),
              replace: true,
            })
          }
        />
      ) : (
        <AgentLibrary
          chrome={chrome}
          projectPath={projectPath}
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
          onSection={chrome.onSection}
          folder={search.folder ?? null}
          onFolder={(folder) =>
            void navigate({
              search: (prev) => ({ ...prev, folder: folder ?? undefined }),
              replace: true,
            })
          }
        />
      )}
      <BundleUpdateDialogs />
    </>
  );
}
