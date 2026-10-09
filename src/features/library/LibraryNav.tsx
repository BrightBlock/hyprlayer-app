import { useRef, useState, type ReactNode, type RefObject } from "react";

import { Icon } from "@/components/Icon";
import { RepoAvatar } from "@/components/RepoAvatar";
import { Button, cx, rowClass, RowSpine } from "@/components/ui";
import type { Project } from "@/lib/bindings";
import { useDocCounts } from "@/lib/queries";
import { countFiltersFromSearch } from "./DocFilters";
import { asLibraryBox, boxCount, LIBRARY_BOX_KEYS, LIBRARY_BOXES, type LibraryBox } from "./libraryBoxes";
import { useLibraryScope } from "./libraryScope";
import type { LibrarySearch } from "./librarySearch";
import { ProjectPicker, type ScopeProject } from "./ProjectPicker";

const PROJECT_ROWS = 5;

export function LibraryNav({
  search,
  onUpdate,
  projects,
  onNewDoc,
}: {
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
  projects: Project[];
  onNewDoc: () => void;
}) {
  const counts = useDocCounts(search.project, countFiltersFromSearch(search)).data;
  const pinned = useLibraryScope((s) => s.pinnedProjects);
  const togglePinned = useLibraryScope((s) => s.togglePinnedProject);
  const box = asLibraryBox(search.box);

  const scopeRef = useRef<HTMLButtonElement>(null);
  const findRef = useRef<HTMLButtonElement>(null);
  const allRef = useRef<HTMLButtonElement>(null);
  const [pickerAnchor, setPickerAnchor] = useState<RefObject<HTMLElement | null> | null>(null);

  const byName = new Map(projects.map((p) => [p.thoughtsProject, p]));
  const scopeProject = (name: string, count: number): ScopeProject => {
    const p = byName.get(name);
    return { name, label: p?.mappedName ?? name, count, originUrl: p?.originUrl ?? null };
  };
  const all = (counts?.projects ?? []).map((n) => scopeProject(n.name, n.count));
  const countOf = new Map(all.map((p) => [p.name, p]));
  const current = search.project ? (countOf.get(search.project) ?? scopeProject(search.project, 0)) : null;

  // Pins first, then the busiest projects; the scoped one always shows so its row can carry the spine.
  const rows = pinned.map((name) => countOf.get(name) ?? scopeProject(name, 0));
  for (const p of all) {
    if (rows.length >= PROJECT_ROWS) break;
    if (!pinned.includes(p.name)) rows.push(p);
  }
  if (current && !rows.some((p) => p.name === current.name)) rows.push(current);

  const setBox = (next: LibraryBox) => onUpdate({ box: next === "all" ? undefined : next });
  const setProject = (name: string | undefined) => onUpdate({ project: name, doc: undefined });

  const boxRows = (group: "smart" | "types" | "status") =>
    LIBRARY_BOX_KEYS.filter((k) => LIBRARY_BOXES[k].group === group).map((k) => (
      <NavRow
        key={k}
        active={box === k}
        onClick={() => setBox(k)}
        leading={
          <Icon name={LIBRARY_BOXES[k].icon} size="sm" className={k === "new" ? "text-info" : undefined} />
        }
        label={LIBRARY_BOXES[k].label}
        trailing={boxCount(k, counts)}
      />
    ));

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 py-2.5">
      <Button
        variant="primary"
        onClick={onNewDoc}
        className="mb-2 mt-0.5 h-8 w-full shrink-0 justify-center text-ui font-semibold"
      >
        <Icon name="edit" size="xs" />
        New thought
      </Button>
      <button
        ref={scopeRef}
        type="button"
        onClick={() => setPickerAnchor(scopeRef)}
        aria-haspopup="dialog"
        aria-expanded={pickerAnchor === scopeRef}
        aria-label="Project scope"
        className="mb-2 flex h-8 w-full shrink-0 items-center gap-2 rounded-control border border-edge bg-panel px-2.5 shadow-sm hover:bg-edge/50 focus:outline-none focus-visible:bg-edge/50"
      >
        <Icon name="folder" size="sm" className="text-ink-dim" />
        <span className="min-w-0 flex-1 truncate text-left text-ui font-semibold text-ink">
          {current?.label ?? "All projects"}
        </span>
        {counts && !current && <span className="shrink-0 text-meta text-ink-4">{counts.projects.length}</span>}
        <Icon name="chevronDown" size="xs" className="text-ink-4" />
      </button>

      <nav aria-label="Library">
        {boxRows("smart")}
        <GroupLabel>Types</GroupLabel>
        {boxRows("types")}
        <GroupLabel>Status</GroupLabel>
        {boxRows("status")}
        <GroupLabel
          trailing={
            <button
              ref={findRef}
              type="button"
              onClick={() => setPickerAnchor(findRef)}
              className="flex h-6 w-6 items-center justify-center rounded-control text-ink-4 hover:bg-edge/60 hover:text-ink"
              title="Find a project"
              aria-label="Find a project"
            >
              <Icon name="search" size="xs" />
            </button>
          }
        >
          Projects
        </GroupLabel>
        {rows.map((p) => (
          <NavRow
            key={p.name}
            active={search.project === p.name}
            onClick={() => setProject(p.name)}
            leading={<RepoAvatar source={p.originUrl} name={p.label} size={14} className="rounded" />}
            label={p.label}
            badge={pinned.includes(p.name) && <Icon name="star" size="xs" className="text-ink-4" label="Pinned" />}
            trailing={p.count}
          />
        ))}
        {all.length > 0 && (
          <NavRow
            ref={allRef}
            active={false}
            onClick={() => setPickerAnchor(allRef)}
            leading={<Icon name="more" size="sm" />}
            label={`All ${all.length} projects`}
            trailing={<Icon name="chevronRight" size="xs" />}
            className="text-ink-dim"
          />
        )}
      </nav>

      <ProjectPicker
        open={pickerAnchor !== null}
        onClose={() => setPickerAnchor(null)}
        anchorRef={pickerAnchor ?? scopeRef}
        projects={all}
        current={search.project}
        pinned={pinned}
        onScope={setProject}
        onTogglePin={togglePinned}
      />
    </div>
  );
}

function GroupLabel({ children, trailing }: { children: ReactNode; trailing?: ReactNode }) {
  return (
    <div className="flex h-8 items-end justify-between gap-2 px-2.5 pb-1">
      <p className="min-w-0 truncate text-ui-sm font-semibold text-ink-dim">{children}</p>
      {trailing}
    </div>
  );
}

function NavRow({
  active,
  onClick,
  leading,
  label,
  badge,
  trailing,
  className,
  ref,
}: {
  active: boolean;
  onClick: () => void;
  leading: ReactNode;
  label: string;
  badge?: ReactNode;
  trailing: ReactNode;
  className?: string;
  ref?: RefObject<HTMLButtonElement | null>;
}) {
  return (
    <button
      ref={ref}
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      className={rowClass({
        tier: "compact",
        selected: active ? "spine" : "none",
        className: cx(
          "flex w-full gap-2 px-2.5 text-left text-ui",
          active ? "font-semibold text-ink" : "text-ink-2",
          className,
        ),
      })}
    >
      {active && <RowSpine />}
      {leading}
      <span className="min-w-0 truncate">{label}</span>
      {badge}
      {trailing !== null && <span className="ml-auto shrink-0 text-meta text-ink-4">{trailing}</span>}
    </button>
  );
}
