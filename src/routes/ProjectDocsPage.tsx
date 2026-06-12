import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { Button, ErrorNote, Field, Select, Spinner, TextInput } from "../components/ui";
import { CreateDocDialog } from "../features/library/CreateDocDialog";
import { DOC_PAGE_SIZE, DocList } from "../features/library/DocList";
import { DOC_SCOPES, DOC_STATUSES, DOC_TYPES } from "../features/library/constants";
import type { ThoughtQuery } from "../lib/bindings";
import { useDocs, useProjectDetail } from "../lib/queries";
import type { ProjectDocsSearch } from "../router";

function splitParam(value: string | undefined): string[] {
  return value
    ? value
        .split(",")
        .map((part) => part.trim())
        .filter(Boolean)
    : [];
}

function joinSingle(value: string): string | undefined {
  return value.length > 0 ? value : undefined;
}

export function ProjectDocsPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath/docs" });
  const search = useSearch({ from: "/projects/$projectPath/docs" });
  const navigate = useNavigate({ from: "/projects/$projectPath/docs" });
  const detail = useProjectDetail(projectPath);
  const [createOpen, setCreateOpen] = useState(false);

  const mappedName = detail.data?.project.mappedName;
  const query: ThoughtQuery = useMemo(
    () => ({
      text: search.q ?? null,
      types: splitParam(search.type),
      statuses: splitParam(search.status),
      projects: mappedName ? [mappedName] : [],
      scopes: splitParam(search.scope),
      authors: splitParam(search.author),
      tags: splitParam(search.tag),
      tagMode: "or",
      sort: search.q ? "relevance" : "date",
      limit: DOC_PAGE_SIZE,
      offset: 0,
    }),
    [mappedName, search],
  );
  const docs = useDocs(query, Boolean(mappedName));

  const update = (patch: Partial<ProjectDocsSearch>) =>
    void navigate({
      search: (prev) => ({ ...prev, ...patch }),
      replace: true,
    });

  if (detail.isLoading) {
    return (
      <div className="flex justify-center p-12">
        <Spinner />
      </div>
    );
  }

  if (detail.isError) {
    return (
      <div className="p-8">
        <ErrorNote error={detail.error} />
      </div>
    );
  }

  const project = detail.data!.project;

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

        <div className="mt-4 space-y-3">
          <Field label="Search">
            <TextInput
              value={search.q ?? ""}
              onChange={(e) => update({ q: joinSingle(e.target.value) })}
              placeholder="Title, tag, or body"
            />
          </Field>
          <Field label="Type">
            <Select value={search.type ?? ""} onChange={(e) => update({ type: joinSingle(e.target.value) })}>
              <option value="">All types</option>
              {DOC_TYPES.map((type) => (
                <option key={type} value={type}>
                  {type}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Status">
            <Select
              value={search.status ?? ""}
              onChange={(e) => update({ status: joinSingle(e.target.value) })}
            >
              <option value="">All statuses</option>
              {DOC_STATUSES.map((status) => (
                <option key={status} value={status}>
                  {status}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Scope">
            <Select value={search.scope ?? ""} onChange={(e) => update({ scope: joinSingle(e.target.value) })}>
              <option value="">All scopes</option>
              {DOC_SCOPES.map((scope) => (
                <option key={scope} value={scope}>
                  {scope}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Author">
            <TextInput
              value={search.author ?? ""}
              onChange={(e) => update({ author: joinSingle(e.target.value) })}
              placeholder="Exact author"
            />
          </Field>
          <Field label="Tag">
            <TextInput
              value={search.tag ?? ""}
              onChange={(e) => update({ tag: joinSingle(e.target.value) })}
              placeholder="Exact tag"
            />
          </Field>
          <Button
            className="w-full justify-center"
            onClick={() =>
              update({
                q: undefined,
                type: undefined,
                status: undefined,
                scope: undefined,
                author: undefined,
                tag: undefined,
              })
            }
          >
            Clear filters
          </Button>
        </div>
      </aside>

      <section className="min-w-0 flex-1 overflow-y-auto">
        <div className="sticky top-0 z-10 border-b border-edge bg-surface/95 px-6 py-4 backdrop-blur">
          <div className="flex items-center justify-between gap-4">
            <div>
              <h2 className="text-sm font-medium">{project.mappedName}</h2>
              <p className="text-sm text-ink-dim">
                {docs.data ? `${docs.data.total} document${docs.data.total === 1 ? "" : "s"}` : "Loading documents"}
              </p>
              {docs.data && docs.data.total > DOC_PAGE_SIZE && (
                <p className="text-xs text-ink-dim">Showing first {DOC_PAGE_SIZE}; narrow the filters to refine.</p>
              )}
            </div>
            <Link
              to="/library"
              search={{ project: project.mappedName }}
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
        initialProjectName={project.mappedName}
      />
    </div>
  );
}
