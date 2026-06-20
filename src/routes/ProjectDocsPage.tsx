import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { Button, PageError, PageSpinner } from "../components/ui";
import { CreateDocDialog } from "../features/library/CreateDocDialog";
import { DocCountSummary, DocFilterFields, docQueryFromSearch } from "../features/library/DocFilters";
import { DocList } from "../features/library/DocList";
import { useBootstrap, useDocs } from "../lib/queries";
import type { ProjectDocsSearch } from "../router";

export function ProjectDocsPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath/docs" });
  const search = useSearch({ from: "/projects/$projectPath/docs" });
  const navigate = useNavigate({ from: "/projects/$projectPath/docs" });
  const bootstrap = useBootstrap();
  const [createOpen, setCreateOpen] = useState(false);

  // The bootstrap cache already names every project — no per-page
  // `storage info` subprocess needed just for the mapped name.
  const project = bootstrap.data?.projects.find((p) => p.path === projectPath);
  // Scope docs by the thoughts key (the doc-index project id), not the display name.
  const projectKey = project?.thoughtsProject;
  const query = useMemo(() => docQueryFromSearch(search, projectKey), [projectKey, search]);
  const docs = useDocs(query, Boolean(projectKey));

  const update = (patch: Partial<ProjectDocsSearch>) =>
    void navigate({
      search: (prev) => ({ ...prev, ...patch }),
      replace: true,
    });

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;
  if (!project) {
    return (
      <PageError
        error={{
          code: "project_not_found",
          message: `No hyprlayer project is mapped at ${projectPath}`,
          remedy: "Add the repository as a project first.",
        }}
      />
    );
  }

  return (
    <div className="flex h-full min-h-0">
      <aside className="w-64 shrink-0 overflow-y-auto border-r border-edge bg-panel p-4">
        <Link
          to="/projects/$projectPath"
          params={{ projectPath }}
          className="text-xs text-ink-dim hover:text-ink"
        >
          {project.mappedName}
        </Link>
        <div className="mt-1 flex items-center justify-between gap-2">
          <h1 className="text-base font-semibold">Project docs</h1>
          <Button variant="primary" onClick={() => setCreateOpen(true)}>
            New
          </Button>
        </div>

        <DocFilterFields search={search} onUpdate={update} />
      </aside>

      <section className="min-w-0 flex-1 overflow-y-auto">
        <div className="sticky top-0 z-10 border-b border-edge bg-surface/95 px-6 py-4 backdrop-blur">
          <div className="flex items-center justify-between gap-4">
            <div>
              <h2 className="text-sm font-medium">{project.mappedName}</h2>
              <DocCountSummary result={docs.data} />
            </div>
            <Link
              to="/library"
              search={{ project: project.thoughtsProject }}
              className="rounded-md border border-edge bg-panel-2 px-3 py-1.5 text-sm font-medium text-ink hover:bg-edge/60"
            >
              Open in Library
            </Link>
          </div>
        </div>

        <div className="p-6">
          <DocList
            result={docs.data}
            isLoading={docs.isLoading}
            isError={docs.isError}
            error={docs.error}
            emptyText="No documents are indexed for this project yet."
          />
        </div>
      </section>

      <CreateDocDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        initialProjectName={project.thoughtsProject}
      />
    </div>
  );
}
