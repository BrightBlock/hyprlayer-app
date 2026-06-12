import { useNavigate, useSearch } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { Button, Field, Select, Spinner, TextInput } from "../components/ui";
import { CreateDocDialog } from "../features/library/CreateDocDialog";
import { DOC_PAGE_SIZE, DocList } from "../features/library/DocList";
import { DOC_SCOPES, DOC_STATUSES, DOC_TYPES } from "../features/library/constants";
import { useBootstrap, useDocs } from "../lib/queries";
import type { ThoughtQuery } from "../lib/bindings";
import type { LibrarySearch } from "../router";

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

export function LibraryPage() {
  const search = useSearch({ from: "/library" });
  const navigate = useNavigate({ from: "/library" });
  const bootstrap = useBootstrap();
  const [createOpen, setCreateOpen] = useState(false);

  const query: ThoughtQuery = useMemo(
    () => ({
      text: search.q ?? null,
      types: splitParam(search.type),
      statuses: splitParam(search.status),
      projects: splitParam(search.project),
      scopes: splitParam(search.scope),
      authors: splitParam(search.author),
      tags: splitParam(search.tag),
      tagMode: "or",
      sort: search.q ? "relevance" : "date",
      limit: DOC_PAGE_SIZE,
      offset: 0,
    }),
    [search],
  );
  const docs = useDocs(query);

  const update = (patch: Partial<LibrarySearch>) =>
    void navigate({
      search: (prev) => ({ ...prev, ...patch }),
      replace: true,
    });

  const activeProject = search.project;
  const projects = bootstrap.data?.projects ?? [];

  return (
    <div className="flex h-full min-h-0">
      <aside className="w-64 shrink-0 overflow-y-auto border-r border-edge bg-panel p-4">
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-base font-semibold">Library</h1>
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
          <Field label="Project">
            <Select
              value={search.project ?? ""}
              onChange={(e) => update({ project: joinSingle(e.target.value) })}
            >
              <option value="">All projects</option>
              {projects.map((project) => (
                <option key={project.path} value={project.mappedName}>
                  {project.mappedName}
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
                project: undefined,
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
              <p className="text-sm text-ink-dim">
                {docs.data ? `${docs.data.total} document${docs.data.total === 1 ? "" : "s"}` : "Loading documents"}
              </p>
              {docs.data && docs.data.total > DOC_PAGE_SIZE && (
                <p className="text-xs text-ink-dim">Showing first {DOC_PAGE_SIZE}; narrow the filters to refine.</p>
              )}
            </div>
            {docs.isFetching && <Spinner />}
          </div>
        </div>

        <div className="p-6">
          <DocList
            result={docs.data}
            isLoading={docs.isLoading}
            isError={docs.isError}
            error={docs.error}
            emptyText="No documents match the current filters."
          />
        </div>
      </section>

      <CreateDocDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        initialProjectName={activeProject}
      />
    </div>
  );
}
