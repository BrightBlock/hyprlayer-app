import { Button, Field, Select, TextInput } from "../../components/ui";
import type { QueryResult, ThoughtQuery } from "../../lib/bindings";
import type { LibrarySearch } from "../../router";
import { DOC_PAGE_SIZE } from "./DocList";
import { DOC_SCOPES, DOC_STATUSES, DOC_TYPES } from "./constants";

export function splitParam(value: string | undefined): string[] {
  return value
    ? value
        .split(",")
        .map((part) => part.trim())
        .filter(Boolean)
    : [];
}

export function joinSingle(value: string): string | undefined {
  return value.length > 0 ? value : undefined;
}

/**
 * The query both doc browsers run, built from URL filter params. A fixed
 * project (the project-docs page) replaces the project param.
 */
export function docQueryFromSearch(search: LibrarySearch, fixedProject?: string): ThoughtQuery {
  return {
    text: search.q ?? null,
    types: splitParam(search.type),
    statuses: splitParam(search.status),
    projects: fixedProject ? [fixedProject] : splitParam(search.project),
    scopes: splitParam(search.scope),
    authors: splitParam(search.author),
    tags: splitParam(search.tag),
    tagMode: "or",
    sort: search.q ? "relevance" : "date",
    limit: DOC_PAGE_SIZE,
    offset: 0,
  };
}

type DocFilterFieldsProps = {
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
  /** Options for the Project select; omitted on project-scoped pages. */
  projectOptions?: { path: string; mappedName: string; thoughtsProject: string }[];
};

/** The filter rail both the Library and project-docs pages share. */
export function DocFilterFields({ search, onUpdate, projectOptions }: DocFilterFieldsProps) {
  return (
    <div className="mt-4 space-y-3">
      <Field label="Search">
        <TextInput
          value={search.q ?? ""}
          onChange={(e) => onUpdate({ q: joinSingle(e.target.value) })}
          placeholder="Title, tag, or body"
        />
      </Field>
      <Field label="Type">
        <Select value={search.type ?? ""} onChange={(e) => onUpdate({ type: joinSingle(e.target.value) })}>
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
          onChange={(e) => onUpdate({ status: joinSingle(e.target.value) })}
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
        <Select value={search.scope ?? ""} onChange={(e) => onUpdate({ scope: joinSingle(e.target.value) })}>
          <option value="">All scopes</option>
          {DOC_SCOPES.map((scope) => (
            <option key={scope} value={scope}>
              {scope}
            </option>
          ))}
        </Select>
      </Field>
      {projectOptions && (
        <Field label="Project">
          <Select
            value={search.project ?? ""}
            onChange={(e) => onUpdate({ project: joinSingle(e.target.value) })}
          >
            <option value="">All projects</option>
            {projectOptions.map((project) => (
              <option key={project.path} value={project.thoughtsProject}>
                {project.mappedName}
              </option>
            ))}
          </Select>
        </Field>
      )}
      <Field label="Author">
        <TextInput
          value={search.author ?? ""}
          onChange={(e) => onUpdate({ author: joinSingle(e.target.value) })}
          placeholder="Exact author"
        />
      </Field>
      <Field label="Tag">
        <TextInput
          value={search.tag ?? ""}
          onChange={(e) => onUpdate({ tag: joinSingle(e.target.value) })}
          placeholder="Exact tag"
        />
      </Field>
      <Button
        className="w-full justify-center"
        onClick={() =>
          onUpdate({
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
  );
}

/** "N documents" + first-page note, shared by both doc browsers' headers. */
export function DocCountSummary({ result }: { result?: QueryResult }) {
  return (
    <>
      <p className="text-sm text-ink-dim">
        {result ? `${result.total} document${result.total === 1 ? "" : "s"}` : "Loading documents"}
      </p>
      {result && result.total > DOC_PAGE_SIZE && (
        <p className="text-xs text-ink-dim">Showing first {DOC_PAGE_SIZE}; narrow the filters to refine.</p>
      )}
    </>
  );
}
