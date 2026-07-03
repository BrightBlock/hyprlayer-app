import { useState } from "react";

import { Badge, ErrorNote, Field, Select, Spinner, TextInput } from "../../components/ui";
import { ChevronDownIcon, ChevronRightIcon, FolderIcon, PlusIcon } from "../../components/icons";
import { RepoAvatar } from "../../components/RepoAvatar";
import type { Project, QueryResult } from "../../lib/bindings";
import type { LibrarySearch } from "../../router";
import { formatRelativeTime } from "../session/toolDisplay";
import { DOC_SCOPES, DOC_STATUSES, DOC_TYPES, STATUS_TONE } from "./constants";
import { groupDocsByType } from "./docGroups";
import { docTitle } from "./text";
import { ThoughtsSyncButton } from "./ThoughtsSyncButton";
import type { ThoughtsSyncState } from "./useThoughtsSync";

type Props = {
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
  projects: Project[];
  result?: QueryResult;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  onNewDoc: () => void;
  /** Thoughts-repo sync state + trigger, rendered as the header's sync icon. */
  sync: ThoughtsSyncState;
  /** Drag-resizable width (px), owned by the route so it survives navigation. */
  width: number;
  /** Collapsed (hidden) state — toggled from the header, like the project panel. */
  collapsed: boolean;
  /** True only while the resize handle is dragged, to suppress the width
   *  transition so the panel tracks the cursor instead of trailing it. */
  resizing: boolean;
};

/** The mock's "LIBRARY" master panel: project switcher, filter input, type tabs,
 *  a collapsed advanced-filters disclosure, and the type-grouped doc list. */
export function LibrarySidebar({
  search,
  onUpdate,
  projects,
  result,
  isLoading,
  isError,
  error,
  onNewDoc,
  sync,
  width,
  collapsed,
  resizing,
}: Props) {
  const groups = groupDocsByType(result?.items ?? []);
  const now = Date.now();

  // Width-animates between 0 and `width` so a collapse/expand glides instead of
  // snapping (matching the project + diff panels). overflow-hidden clips the
  // fixed-width inner shell so its content never reflows mid-animation; the
  // transition is dropped while the resize handle is dragged. It is not gated on
  // reduced motion — Windows maps that to the often-off "Animation effects"
  // toggle, which would otherwise snap the panel.
  return (
    <aside
      className={`relative shrink-0 overflow-hidden border-r border-edge bg-panel ${
        resizing ? "" : "transition-[width] duration-200 ease-out"
      }`}
      style={{ width: collapsed ? 0 : width }}
      inert={collapsed}
    >
      <div className="absolute inset-y-0 left-0 flex flex-col" style={{ width }}>
      <div className="flex h-11 shrink-0 items-center justify-between border-b border-edge px-4">
        <p className="text-xs font-medium uppercase tracking-wider text-ink-dim">Library</p>
        <div className="flex items-center gap-1">
          <ThoughtsSyncButton sync={sync} />
          <button
            type="button"
            onClick={onNewDoc}
            className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
            title="New document"
            aria-label="New document"
          >
            <PlusIcon size={14} />
          </button>
        </div>
      </div>

      <div className="shrink-0 space-y-2 border-b border-edge p-3">
        <ProjectSwitcher
          projects={projects}
          value={search.project}
          total={result?.total}
          // Changing project scope drops the open doc: a doc is project-scoped,
          // so keeping it would leave the detail pane on another project's
          // document while the sidebar reads the new scope.
          onChange={(project) => onUpdate({ project, doc: undefined, mode: undefined })}
        />
        <TextInput
          value={search.q ?? ""}
          onChange={(e) => onUpdate({ q: e.target.value || undefined })}
          placeholder="Filter…"
        />
        <TypeTabs value={search.type} onChange={(type) => onUpdate({ type })} />
        <MoreFilters search={search} onUpdate={onUpdate} />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        {isError && (
          <div className="p-3">
            <ErrorNote error={error} />
          </div>
        )}
        {isLoading && (
          <div className="flex justify-center p-8">
            <Spinner />
          </div>
        )}
        {result && result.items.length === 0 && (
          <p className="px-4 py-6 text-sm text-ink-dim">No documents match the current filters.</p>
        )}
        {groups.map((group) => (
          <div key={group.type || "untyped"} className="mb-1">
            <div className="flex items-center justify-between px-4 pb-1 pt-2">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim/70">
                {group.label}
              </span>
              <span className="text-[10px] text-ink-dim/50">
                {formatRelativeTime(group.newestMs, now).replace(" ago", "")}
              </span>
            </div>
            {group.docs.map((doc) => {
              const selected = search.doc === doc.id;
              const subtitle = [doc.project ?? doc.fm.project, doc.fm.author]
                .filter(Boolean)
                .join(" · ");
              const related = doc.fm.related?.length ?? 0;
              return (
                <button
                  key={doc.id}
                  type="button"
                  onClick={() => onUpdate({ doc: doc.id, mode: undefined })}
                  className={`relative block w-full px-4 py-2 text-left ${
                    selected ? "bg-accent/15" : "hover:bg-edge/40"
                  }`}
                >
                  {selected && (
                    <span className="absolute inset-y-0 left-0 w-[3px] bg-accent" aria-hidden="true" />
                  )}
                  <p
                    className={`truncate text-sm ${selected ? "font-semibold text-ink" : "font-medium text-ink"}`}
                  >
                    {docTitle(doc)}
                  </p>
                  {subtitle && (
                    <p className="mt-0.5 truncate text-xs text-ink-dim">{subtitle}</p>
                  )}
                  <div className="mt-1 flex items-center gap-2 text-[11px] text-ink-dim">
                    {related > 0 && <span>≡ {related}</span>}
                    {doc.fm.status && (
                      <Badge tone={STATUS_TONE[doc.fm.status] ?? "dim"}>{doc.fm.status}</Badge>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        ))}
      </div>
      </div>
    </aside>
  );
}

/** Scope dropdown: "All projects" or one project (sets the `project` thoughts key). */
function ProjectSwitcher({
  projects,
  value,
  total,
  onChange,
}: {
  projects: Project[];
  value: string | undefined;
  total: number | undefined;
  onChange: (project: string | undefined) => void;
}) {
  const [open, setOpen] = useState(false);
  const current = projects.find((p) => p.thoughtsProject === value);

  const choose = (project: string | undefined) => {
    onChange(project);
    setOpen(false);
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-sm hover:bg-edge/40"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {current ? (
          <RepoAvatar source={current.originUrl} name={current.mappedName} size={16} className="rounded" />
        ) : (
          <FolderIcon size={14} />
        )}
        <span className="min-w-0 flex-1 truncate text-left font-medium text-ink">
          {current?.mappedName ?? "All projects"}
        </span>
        {typeof total === "number" && (
          <span className="shrink-0 text-xs tabular-nums text-ink-dim/70">{total}</span>
        )}
        <ChevronDownIcon size={12} className="shrink-0 text-ink-dim" />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={() => setOpen(false)} aria-hidden="true" />
          <div className="absolute left-0 right-0 top-full z-30 mt-1 max-h-72 overflow-y-auto rounded-lg border border-edge bg-panel p-1 shadow-xl">
            <SwitcherItem active={!value} onClick={() => choose(undefined)}>
              <FolderIcon size={14} />
              <span className="flex-1 truncate">All projects</span>
            </SwitcherItem>
            {projects.map((p) => (
              <SwitcherItem
                key={p.path}
                active={p.thoughtsProject === value}
                onClick={() => choose(p.thoughtsProject)}
              >
                <RepoAvatar source={p.originUrl} name={p.mappedName} size={14} className="rounded" />
                <span className="flex-1 truncate">{p.mappedName}</span>
              </SwitcherItem>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function SwitcherItem({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${
        active ? "bg-accent/15 text-ink" : "text-ink-dim hover:bg-edge/50 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

/** ALL + each canonical type, horizontally scrollable; single-select. */
function TypeTabs({
  value,
  onChange,
}: {
  value: string | undefined;
  onChange: (type: string | undefined) => void;
}) {
  const tab = (label: string, active: boolean, onClick: () => void) => (
    <button
      key={label}
      type="button"
      onClick={onClick}
      className={`shrink-0 rounded px-2 py-1 text-[11px] font-semibold uppercase tracking-wider ${
        active ? "bg-accent/20 text-accent" : "text-ink-dim hover:bg-edge/40 hover:text-ink"
      }`}
    >
      {label}
    </button>
  );
  return (
    <div className="no-scrollbar flex items-center gap-1 overflow-x-auto">
      {tab("All", !value, () => onChange(undefined))}
      {DOC_TYPES.map((t) => tab(t, value === t, () => onChange(t)))}
    </div>
  );
}

/** Collapsed disclosure for the less-used filters (status/scope/author/tag). */
function MoreFilters({
  search,
  onUpdate,
}: {
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
}) {
  const active = [search.status, search.scope, search.author, search.tag].filter(Boolean).length;
  const [open, setOpen] = useState(active > 0);
  const single = (value: string) => value || undefined;

  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-1 rounded px-1 py-0.5 text-[11px] font-medium uppercase tracking-wider text-ink-dim hover:text-ink"
        aria-expanded={open}
      >
        <ChevronRightIcon size={12} className={`transition-transform ${open ? "rotate-90" : ""}`} />
        Filters{active > 0 ? ` · ${active}` : ""}
      </button>
      {open && (
        <div className="mt-2 space-y-2">
          <Field label="Status">
            <Select
              value={search.status ?? ""}
              onChange={(e) => onUpdate({ status: single(e.target.value) })}
            >
              <option value="">All statuses</option>
              {DOC_STATUSES.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Scope">
            <Select
              value={search.scope ?? ""}
              onChange={(e) => onUpdate({ scope: single(e.target.value) })}
            >
              <option value="">All scopes</option>
              {DOC_SCOPES.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Author">
            <TextInput
              value={search.author ?? ""}
              onChange={(e) => onUpdate({ author: single(e.target.value) })}
              placeholder="Exact author"
            />
          </Field>
          <Field label="Tag">
            <TextInput
              value={search.tag ?? ""}
              onChange={(e) => onUpdate({ tag: single(e.target.value) })}
              placeholder="Exact tag"
            />
          </Field>
        </div>
      )}
    </div>
  );
}
