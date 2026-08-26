
import { useMemo, useState } from "react";

import {
  Badge,
  Button,
  Dialog,
  DropdownSelect,
  EmptyState,
  TextInput,
  type BadgeTone,
} from "@/components/ui";
import { ArchiveIcon, GitBranchIcon, ModeIcon, SearchIcon, UnarchiveIcon, XIcon } from "@/components/icons";
import type { ShellTab } from "./workstreamTabs";
import { engineLabel } from "@/features/multiplexer/engines";
import { modelOption } from "@/lib/domain/models";
import { formatRelativeTime } from "@/lib/format";
import type { ArchivedShell, ArchivedStatus } from "./workstreamTabs";

type SortKey = "recent" | "name";

const STATUS: Record<ArchivedStatus, { label: string; tone: BadgeTone }> = {
  done: { label: "Done", tone: "ok" },
  stopped: { label: "Stopped", tone: "warn" },
  idle: { label: "Idle", tone: "dim" },
};

function modelLabel(shell: ShellTab): string {
  if (shell.ptyAgent) return engineLabel(shell.ptyAgent.engineId);
  if (shell.harness === "opencode") return shell.opencodeModel ?? "OpenCode";
  if (shell.harness === "codex") return shell.codexModel ?? "Codex";
  return modelOption(shell.model).label;
}

function ArchivedMeta({ shell, branch, archivedAt }: { shell: ShellTab; branch: string; archivedAt: number }) {
  return (
    <div className="mt-0.5 flex min-w-0 items-center gap-2 text-xs text-ink-dim">
      <span className="inline-flex shrink-0 items-center gap-1">
        <span className="h-1.5 w-1.5 rounded-full bg-brand" />
        {modelLabel(shell)}
      </span>
      <span className="inline-flex min-w-0 items-center gap-1 font-mono">
        <GitBranchIcon size={11} />
        <span className="truncate">{branch}</span>
      </span>
      <span className="shrink-0">· archived {formatRelativeTime(archivedAt)}</span>
    </div>
  );
}

type ArchivedAgentsViewProps = {
  workstreamTitle: string;
  branch: string;
  archived: ArchivedShell[];
  onUnarchive: (shellId: number) => void;
  onRemove: (shellId: number) => void;
};

export function ArchivedAgentsView({
  workstreamTitle,
  branch,
  archived,
  onUnarchive,
  onRemove,
}: ArchivedAgentsViewProps) {
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("recent");
  const [confirmRemove, setConfirmRemove] = useState<ArchivedShell | null>(null);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matched = q
      ? archived.filter(
          (a) =>
            a.shell.title.toLowerCase().includes(q) || modelLabel(a.shell).toLowerCase().includes(q),
        )
      : archived;
    const sorted = [...matched];
    if (sort === "recent") sorted.sort((a, b) => b.archivedAt - a.archivedAt);
    else sorted.sort((a, b) => a.shell.title.localeCompare(b.shell.title));
    return sorted;
  }, [archived, query, sort]);

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-panel">
      <div className="flex shrink-0 items-start justify-between gap-4 px-7 pb-4 pt-5">
        <div className="min-w-0">
          <div className="flex items-center gap-2.5">
            <ArchiveIcon size={18} className="text-accent" />
            <h1 className="text-base font-semibold text-ink">Archived agents</h1>
            <span className="truncate text-sm text-ink-dim">· {workstreamTitle}</span>
            <Badge tone="dim">{archived.length}</Badge>
          </div>
          <p className="mt-1.5 max-w-xl text-xs leading-relaxed text-ink-dim">
            Agents you archive in this workstream leave the tab strip but keep their branch,
            transcript and worktree. Unarchive any time to pick up where it left off.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <div className="relative w-52">
            <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim">
              <SearchIcon size={14} />
            </span>
            <TextInput
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search archived…"
              className="pl-8"
              aria-label="Search archived agents"
            />
          </div>
          <div className="w-36">
            <DropdownSelect
              value={sort}
              onChange={(v) => setSort(v as SortKey)}
              aria-label="Sort archived agents"
              options={[
                { value: "recent", label: "Last archived" },
                { value: "name", label: "Name" },
              ]}
            />
          </div>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-7 pb-6">
        {archived.length === 0 ? (
          <EmptyState title="No archived agents">
            Closing an agent&apos;s tab archives it here, with its branch, transcript and worktree
            intact.
          </EmptyState>
        ) : visible.length === 0 ? (
          <EmptyState title="No matches">No archived agents match “{query}”.</EmptyState>
        ) : (
          <div className="divide-y divide-edge/60 overflow-hidden rounded-xl border border-edge bg-panel-2">
            {visible.map((a) => (
              <div key={a.shell.id} className="flex items-center gap-3.5 px-4 py-3.5">
                <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-edge bg-panel text-ink-dim">
                  <ModeIcon mode={a.shell.mode} size={15} />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-ink" title={a.shell.title}>
                    {a.shell.title}
                  </div>
                  <ArchivedMeta shell={a.shell} branch={branch} archivedAt={a.archivedAt} />
                </div>
                <Badge tone={STATUS[a.status].tone}>{STATUS[a.status].label}</Badge>
                <Button onClick={() => onUnarchive(a.shell.id)}>
                  <UnarchiveIcon size={14} />
                  Unarchive
                </Button>
                <button
                  type="button"
                  onClick={() => setConfirmRemove(a)}
                  className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-bad/10 hover:text-bad"
                  title={`Delete ${a.shell.title}`}
                  aria-label={`Delete ${a.shell.title}`}
                >
                  <XIcon size={15} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      <Dialog
        open={Boolean(confirmRemove)}
        onClose={() => setConfirmRemove(null)}
        title="Delete archived agent?"
      >
        <p className="text-sm text-ink-dim">
          <span className="text-ink">{confirmRemove?.shell.title}</span> will be deleted from this
          workstream's archive and can no longer be unarchived. Its branch and worktree belong to
          the workstream and stay put.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <Button onClick={() => setConfirmRemove(null)}>Cancel</Button>
          <Button
            variant="danger"
            onClick={() => {
              if (confirmRemove) onRemove(confirmRemove.shell.id);
              setConfirmRemove(null);
            }}
          >
            Delete
          </Button>
        </div>
      </Dialog>
    </div>
  );
}
