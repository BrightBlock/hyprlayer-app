import { Icon } from "@/components/Icon";
import { cx, rowClass, SegmentedControl, TextInput } from "@/components/ui";
import type { ReviewFile } from "@/lib/bindings";
import { CHANGE_GLYPH } from "@/lib/domain/review";
import { plural } from "@/lib/format";
import { basename, dirname } from "@/lib/path";
import type { PlanDocPhase } from "@/features/plan/planDoc";

import { tasksForFile, type Attribution } from "./attribution";
import type { ReviewGroupBy } from "./reviewStore";

export type TreeGroup = {
  key: string;
  kind: "folder" | "task";
  title: string | null;
  detail: string | null;
  files: ReviewFile[];
};

const byPath = (a: ReviewFile, b: ReviewFile) => a.path.localeCompare(b.path);

// Root files first, then folders by path; the order prev/next and "i of N" walk.
export function folderGroups(files: ReviewFile[]): TreeGroup[] {
  const byDir = new Map<string, ReviewFile[]>();
  for (const f of files) {
    const dir = dirname(f.path);
    const list = byDir.get(dir);
    if (list) list.push(f);
    else byDir.set(dir, [f]);
  }
  return [...byDir.entries()]
    .sort(([a], [b]) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
    .map(([dir, list]) => ({
      key: dir,
      kind: "folder",
      title: dir ? `${dir}/` : null,
      detail: null,
      files: list.sort(byPath),
    }));
}

type TaskGroup = TreeGroup & { phaseLine: string | null; optional: boolean };

// Plan order, then unmatched todos, then "Other changes"; a file sits under the first task that touched it.
export function taskGroups(files: ReviewFile[], attribution: Attribution, phases: PlanDocPhase[]): TreeGroup[] {
  const groups = new Map<string, TaskGroup>();
  const add = (g: Omit<TaskGroup, "kind" | "detail" | "files">) => {
    if (!groups.has(g.key)) groups.set(g.key, { ...g, kind: "task", detail: null, files: [] });
  };
  for (const p of phases) {
    const phaseLine = `Phase ${p.label} · ${p.name}`;
    add({ key: `phase:${p.label}`, title: null, phaseLine, optional: p.tasks.length > 0 });
    for (const t of p.tasks) add({ key: `task:${t.key}`, title: t.title, phaseLine, optional: false });
  }
  const phaseNames = new Map(phases.map((p) => [`phase:${p.label}`, p.name || `Phase ${p.label}`]));
  const other: ReviewFile[] = [];
  for (const f of [...files].sort(byPath)) {
    const ref = tasksForFile(attribution, f)[0];
    if (!ref) {
      other.push(f);
      continue;
    }
    const key =
      ref.phaseLabel === null ? `todo:${ref.subject}` : ref.taskKey ? `task:${ref.taskKey}` : `phase:${ref.phaseLabel}`;
    add({ key, title: ref.subject, phaseLine: null, optional: false });
    const g = groups.get(key)!;
    g.title ??= ref.subject;
    g.files.push(f);
  }
  const out: TreeGroup[] = [];
  for (const g of groups.values()) {
    if (g.optional && g.files.length === 0) continue;
    const count = g.files.length > 0 ? plural(g.files.length, "file") : null;
    out.push({
      key: g.key,
      kind: "task",
      title: g.title ?? phaseNames.get(g.key) ?? null,
      detail: [g.phaseLine, count].filter(Boolean).join(" · ") || null,
      files: g.files,
    });
  }
  if (other.length > 0) out.push({ key: "other", kind: "task", title: "Other changes", detail: null, files: other });
  return out;
}

const GROUP_BY_OPTIONS: { value: ReviewGroupBy; label: string }[] = [
  { value: "task", label: "Plan task" },
  { value: "folder", label: "Folder" },
];

export function ViewedRing({
  viewed,
  onToggle,
  className,
}: {
  viewed: boolean;
  onToggle: () => void;
  className?: string;
}) {
  const label = viewed ? "Mark not viewed" : "Mark viewed";
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={viewed}
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
      className={cx(
        "flex h-5 w-5 shrink-0 items-center justify-center rounded-control hover:bg-edge/70",
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cx(
          "block h-2.5 w-2.5 rounded-full border",
          viewed ? "border-ok bg-ok" : "border-ink-4",
        )}
      />
    </button>
  );
}

function GroupHeader({ group }: { group: TreeGroup }) {
  if (group.kind === "folder") {
    return group.title ? (
      <div className="truncate px-2 pb-1 pt-2.5 text-meta text-ink-4" title={group.title}>
        {group.title}
      </div>
    ) : null;
  }
  return (
    <div className="px-2 pb-1.5 pt-2.5">
      <div className="truncate text-ui font-medium text-ink-2" title={group.title ?? undefined}>
        {group.title}
      </div>
      {group.detail && <div className="truncate text-meta text-ink-4">{group.detail}</div>}
    </div>
  );
}

function FileRow({
  file,
  selected,
  viewed,
  count,
  onSelect,
  onToggleViewed,
}: {
  file: ReviewFile;
  selected: boolean;
  viewed: boolean;
  count: number;
  onSelect: () => void;
  onToggleViewed: () => void;
}) {
  const glyph = CHANGE_GLYPH[file.change];
  const name = file.oldPath
    ? `${basename(file.oldPath)} → ${basename(file.path)}`
    : basename(file.path);
  return (
    <div
      role="button"
      tabIndex={0}
      aria-current={selected ? "true" : undefined}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== " ") return;
        e.preventDefault();
        onSelect();
      }}
      title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
      className={rowClass({
        tier: "compact",
        selected: selected ? "tint" : "none",
        className: "flex gap-2 pl-2 pr-1.5 text-left",
      })}
    >
      <span className={cx("flex min-w-0 flex-1 items-center gap-2", viewed && "opacity-55")}>
        <span className={cx("w-3 shrink-0 text-center text-meta font-semibold", glyph.cls)}>
          {glyph.ch}
        </span>
        <span className="min-w-0 flex-1 truncate text-meta text-ink">{name}</span>
        {!file.binary && (
          <span className="shrink-0 text-meta tabular-nums">
            <span className="text-diff-add">+{file.additions}</span>{" "}
            <span className="text-diff-del">−{file.deletions}</span>
          </span>
        )}
      </span>
      {count > 0 && (
        <span
          className="flex shrink-0 items-center gap-0.5 rounded-full border border-edge px-1.5 text-meta tabular-nums text-ink-2"
          title={plural(count, "open item")}
        >
          <Icon name="comment" size="xs" />
          {count}
        </span>
      )}
      <ViewedRing viewed={viewed} onToggle={onToggleViewed} />
    </div>
  );
}

export function FileTree({
  groups,
  changed,
  selected,
  viewed,
  counts,
  filter,
  groupBy,
  onFilter,
  onGroupBy,
  onSelect,
  onToggleViewed,
}: {
  groups: TreeGroup[];
  // Listed above the groups as well; the walk order comes from `groups` alone.
  changed: ReviewFile[];
  selected: string | null;
  viewed: ReadonlySet<string>;
  counts: ReadonlyMap<string, number>;
  filter: string;
  // Null hides the toggle: without tracked todos there is nothing to group by.
  groupBy: ReviewGroupBy | null;
  onFilter: (value: string) => void;
  onGroupBy: (value: ReviewGroupBy) => void;
  onSelect: (path: string) => void;
  onToggleViewed: (file: ReviewFile) => void;
}) {
  const needle = filter.trim().toLowerCase();
  const all: TreeGroup[] =
    changed.length > 0
      ? [{ key: "changed", kind: "task", title: "Changed since your review", detail: null, files: changed }, ...groups]
      : groups;
  const shown = needle
    ? all
        .map((g) => ({ ...g, files: g.files.filter((f) => f.path.toLowerCase().includes(needle)) }))
        .filter((g) => g.files.length > 0)
    : all;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="shrink-0 space-y-2 border-b border-edge-2 px-3 pb-2 pt-2.5">
        <TextInput
          value={filter}
          onChange={(e) => onFilter(e.target.value)}
          placeholder="Filter files"
          aria-label="Filter files"
          className="h-7 py-0 text-ui"
        />
        {groupBy && (
          <div className="flex items-center gap-2">
            <span className="text-ui text-ink-dim">Group by</span>
            <SegmentedControl aria-label="Group by" value={groupBy} options={GROUP_BY_OPTIONS} onChange={onGroupBy} />
          </div>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-1 pb-2.5 pt-0.5">
        {shown.length === 0 ? (
          <p className="px-2 py-1 text-ui text-ink-dim">No matching files</p>
        ) : (
          shown.map((g) => (
            <div key={g.key}>
              <GroupHeader group={g} />
              {g.kind === "task" && g.files.length === 0 && (
                <p className="pb-1 pl-8 pr-3 text-ui text-ink-4">No code changes for this task.</p>
              )}
              {g.files.map((f) => (
                <FileRow
                  key={f.path}
                  file={f}
                  selected={f.path === selected}
                  viewed={viewed.has(f.path)}
                  count={counts.get(f.path) ?? 0}
                  onSelect={() => onSelect(f.path)}
                  onToggleViewed={() => onToggleViewed(f)}
                />
              ))}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
