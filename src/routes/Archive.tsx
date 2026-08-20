import { useNavigate } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import {
  Button,
  ContextMenu,
  ContextMenuItem,
  Dialog,
  EmptyState,
  ErrorNote,
  PageError,
  PageSpinner,
  Spinner,
  TextInput,
} from "@/components/ui";
import { RepoAvatar } from "@/components/RepoAvatar";
import {
  useArchivedWorkstreams,
  useBootstrap,
  useCleanupArchivedWorkstreams,
  useDeleteArchivedWorkstream,
  useRestoreWorkstream,
} from "@/lib/queries";
import { isAppError } from "@/lib/ipc";
import type { ArchivedWorkstreamView, Project } from "@/lib/bindings";

const MS_PER_DAY = 86_400_000;

/** Coarse buckets the user can reason about at a glance: today, yesterday,
 *  N days, weeks, months, years. The labels match the screenshot reference. */
type Bucket = { label: string; order: number; views: ArchivedWorkstreamView[] };

function bucketFor(archivedAt: number, now: number): { label: string; order: number } {
  const ms = now - archivedAt;
  const days = Math.floor(ms / MS_PER_DAY);
  if (days <= 0) return { label: "Today", order: 0 };
  if (days === 1) return { label: "Yesterday", order: 1 };
  if (days < 7) return { label: `${days} days ago`, order: 10 + days };
  if (days < 30) {
    const w = Math.floor(days / 7);
    return { label: w === 1 ? "1 week ago" : `${w} weeks ago`, order: 100 + w };
  }
  if (days < 365) {
    const m = Math.floor(days / 30);
    return { label: m === 1 ? "1 month ago" : `${m} months ago`, order: 1_000 + m };
  }
  const y = Math.floor(days / 365);
  return { label: y === 1 ? "1 year ago" : `${y} years ago`, order: 10_000 + y };
}

function formatDate(ms: number): string {
  // Short, locale-aware, matches the screenshot's "Jun 19" / "Apr 30" style.
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function projectName(projects: Project[], path: string): string {
  return projects.find((p) => p.path === path)?.mappedName ?? path.split("/").pop() ?? path;
}

function projectOriginUrl(projects: Project[], path: string): string | null {
  return projects.find((p) => p.path === path)?.originUrl ?? null;
}

export function Archive() {
  const archived = useArchivedWorkstreams();
  const bootstrap = useBootstrap();
  const restore = useRestoreWorkstream();
  const purge = useDeleteArchivedWorkstream();
  const cleanup = useCleanupArchivedWorkstreams();
  const navigate = useNavigate();

  const [filter, setFilter] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; view: ArchivedWorkstreamView } | null>(
    null,
  );
  const [confirmDelete, setConfirmDelete] = useState<ArchivedWorkstreamView | null>(null);
  const [cleanupOpen, setCleanupOpen] = useState(false);
  const [restoreError, setRestoreError] = useState<unknown>(null);

  // Memoise so the bucket-grouping useMemo doesn't re-run on every render
  // just because we built a fresh empty array each time.
  const projects = useMemo<Project[]>(
    () => bootstrap.data?.projects ?? [],
    [bootstrap.data?.projects],
  );

  const buckets = useMemo<Bucket[]>(() => {
    if (!archived.data) return [];
    const now = Date.now();
    const needle = filter.trim().toLowerCase();
    const groups = new Map<string, Bucket>();
    for (const view of archived.data) {
      // archivedAt is always set on archived rows; the chain is just to satisfy
    // the binding's `number | null` (specta widens f64 to nullable in TS).
    const at: number = view.item.archivedAt ?? view.item.updatedAt ?? 0;
      if (needle) {
        const haystack = `${view.item.title} ${view.item.branch} ${projectName(projects, view.item.projectPath)}`.toLowerCase();
        if (!haystack.includes(needle)) continue;
      }
      const { label, order } = bucketFor(at, now);
      let bucket = groups.get(label);
      if (!bucket) {
        bucket = { label, order, views: [] };
        groups.set(label, bucket);
      }
      bucket.views.push(view);
    }
    return [...groups.values()].sort((a, b) => a.order - b.order);
  }, [archived.data, filter, projects]);

  if (archived.isLoading) return <PageSpinner />;
  if (archived.isError) return <PageError error={archived.error} />;

  const onRestore = (view: ArchivedWorkstreamView) => {
    setMenu(null);
    setRestoreError(null);
    restore.mutate(view.item.id, {
      onSuccess: (restored) => {
        // Drop the user straight into the restored workstream.
        void navigate({
          to: "/projects/$projectPath",
          params: { projectPath: restored.item.projectPath },
          search: { item: restored.item.id },
        });
      },
      onError: (err) => setRestoreError(err),
    });
  };

  const onPurge = (view: ArchivedWorkstreamView) => {
    setMenu(null);
    setConfirmDelete(view);
  };

  const total = archived.data?.length ?? 0;
  const shown = buckets.reduce((acc, b) => acc + b.views.length, 0);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="sticky top-0 z-10 border-b border-edge bg-surface/95 px-6 py-4 backdrop-blur">
        <div className="flex items-center justify-between gap-4">
          <h1 className="text-base font-semibold">Archived workstreams</h1>
          <div className="flex items-center gap-2">
            {(restore.isPending || purge.isPending || cleanup.isPending || archived.isFetching) && (
              <Spinner />
            )}
            <Button
              type="button"
              variant="ghost"
              disabled={total === 0}
              onClick={() => setCleanupOpen(true)}
            >
              Clean up…
            </Button>
          </div>
        </div>
        <div className="mt-3">
          <TextInput
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter by name, branch, or project…"
            aria-label="Filter archived workstreams"
          />
        </div>
        {restoreError ? (
          <div className="mt-3">
            <ErrorNote error={restoreError} onDismiss={() => setRestoreError(null)} />
          </div>
        ) : null}
      </div>

      <div className="min-w-0 flex-1 overflow-y-auto px-6 py-4">
        {total === 0 ? (
          <EmptyState title="No archived workstreams yet" />
        ) : shown === 0 ? (
          <EmptyState title="No matches">
            Nothing in the archive matches{" "}
            <span className="font-mono text-xs">{filter}</span>.
          </EmptyState>
        ) : (
          <div className="space-y-6">
            {buckets.map((bucket) => (
              <section key={bucket.label}>
                <header className="flex items-baseline gap-2 px-2 pb-2 text-xs uppercase tracking-wide text-ink-dim">
                  <span>{bucket.label}</span>
                  <span className="opacity-60">{bucket.views.length}</span>
                </header>
                <ul className="divide-y divide-edge/60 rounded-md border border-edge bg-panel">
                  {bucket.views.map((view) => (
                    <ArchivedRow
                      key={view.item.id}
                      view={view}
                      projectLabel={projectName(projects, view.item.projectPath)}
                      projectOrigin={projectOriginUrl(projects, view.item.projectPath)}
                      busy={restore.isPending || purge.isPending}
                      onRestore={() => onRestore(view)}
                      onMenu={(e) => {
                        e.preventDefault();
                        setMenu({ x: e.clientX, y: e.clientY, view });
                      }}
                    />
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </div>

      {menu && (
        <ContextMenu position={{ x: menu.x, y: menu.y }} onClose={() => setMenu(null)}>
          <ContextMenuItem
            onClick={() => onRestore(menu.view)}
            disabled={!menu.view.archiveExists}
          >
            Unarchive
          </ContextMenuItem>
          <ContextMenuItem danger onClick={() => onPurge(menu.view)}>
            Delete permanently…
          </ContextMenuItem>
        </ContextMenu>
      )}

      <PurgeConfirmDialog
        target={confirmDelete}
        busy={purge.isPending}
        onClose={() => {
          if (!purge.isPending) setConfirmDelete(null);
        }}
        onConfirm={() => {
          if (!confirmDelete) return;
          purge.mutate(confirmDelete.item.id, {
            onSettled: () => setConfirmDelete(null),
          });
        }}
        error={purge.isError ? purge.error : null}
      />

      <CleanupDialog
        open={cleanupOpen}
        archived={archived.data ?? []}
        busy={cleanup.isPending}
        onClose={() => {
          if (!cleanup.isPending) setCleanupOpen(false);
        }}
        onConfirm={(beforeMs) => {
          cleanup.mutate(beforeMs, { onSettled: () => setCleanupOpen(false) });
        }}
        error={cleanup.isError ? cleanup.error : null}
      />
    </div>
  );
}

type RowProps = {
  view: ArchivedWorkstreamView;
  projectLabel: string;
  projectOrigin: string | null;
  busy: boolean;
  onRestore: () => void;
  onMenu: (e: React.MouseEvent) => void;
};

function ArchivedRow({ view, projectLabel, projectOrigin, busy, onRestore, onMenu }: RowProps) {
  const archivedAt: number = view.item.archivedAt ?? view.item.updatedAt ?? 0;
  const missingTitle = view.archiveExists
    ? `${view.item.title} · ${view.item.branch}`
    : `${view.item.title} · backup directory missing`;
  return (
    <li
      className="group flex items-center gap-3 px-3 py-2 hover:bg-edge/40"
      onContextMenu={onMenu}
      title={missingTitle}
    >
      <RepoAvatar source={projectOrigin} name={projectLabel} size={18} />
      <span className="w-32 shrink-0 truncate text-sm text-ink-dim" title={projectLabel}>
        {projectLabel}
      </span>
      <span className="min-w-0 flex-1 truncate text-sm text-ink">{view.item.title}</span>
      <span className="hidden truncate font-mono text-xs text-ink-dim md:inline-block md:max-w-[18rem]">
        {view.item.branch}
      </span>
      {!view.archiveExists && (
        <span
          className="rounded-full bg-warn/20 px-2 py-0.5 text-[10px] uppercase tracking-wide text-warn"
          title={`Backup directory was deleted from ${view.archivePath}`}
        >
          missing
        </span>
      )}
      <span className="w-16 shrink-0 text-right text-xs text-ink-dim">
        {formatDate(archivedAt)}
      </span>
      <Button
        type="button"
        variant="ghost"
        onClick={(e) => {
          e.stopPropagation();
          onRestore();
        }}
        disabled={busy || !view.archiveExists}
        className="opacity-0 group-hover:opacity-100"
      >
        Unarchive
      </Button>
    </li>
  );
}

type ConfirmProps = {
  target: ArchivedWorkstreamView | null;
  busy: boolean;
  onClose: () => void;
  onConfirm: () => void;
  error: unknown;
};

type CleanupScope = "all" | 30 | 60 | 90;

type CleanupProps = {
  open: boolean;
  archived: ArchivedWorkstreamView[];
  busy: boolean;
  onClose: () => void;
  onConfirm: (beforeMs: number | null) => void;
  error: unknown;
};

function CleanupDialog({ open, archived, busy, onClose, onConfirm, error }: CleanupProps) {
  const [scope, setScope] = useState<CleanupScope>("all");

  const now = Date.now();
  const beforeMs: number | null =
    scope === "all" ? null : now - scope * MS_PER_DAY;

  const count =
    scope === "all"
      ? archived.length
      : archived.filter((v) => {
          const at: number = v.item.archivedAt ?? v.item.updatedAt ?? 0;
          return at < (beforeMs as number);
        }).length;

  const scopeOptions: Array<{ value: CleanupScope; label: string }> = [
    { value: "all", label: "All archives" },
    { value: 30, label: "Older than 30 days" },
    { value: 60, label: "Older than 60 days" },
    { value: 90, label: "Older than 90 days" },
  ];

  return (
    <Dialog open={open} onClose={onClose} title="Clean up archives">
      <div className="space-y-4">
        <p className="text-sm text-ink-dim">
          Deletes them from disk, along with their local branches if your Git settings say so.
          This cannot be undone.
        </p>

        <div className="space-y-1.5" role="radiogroup" aria-label="Cleanup scope">
          {scopeOptions.map((opt) => (
            <button
              key={String(opt.value)}
              type="button"
              role="radio"
              aria-checked={scope === opt.value}
              onClick={() => setScope(opt.value)}
              className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-edge/40"
            >
              <span
                className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full border ${
                  scope === opt.value ? "border-accent" : "border-edge"
                }`}
              >
                {scope === opt.value && <span className="h-2 w-2 rounded-full bg-accent" />}
              </span>
              {opt.label}
            </button>
          ))}
        </div>

        {error && isAppError(error) ? <ErrorNote error={error} /> : null}

        <div className="flex items-center justify-end gap-2">
          <Button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="danger"
            busy={busy}
            disabled={count === 0}
            onClick={() => onConfirm(beforeMs)}
          >
            Delete {count} permanently
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function PurgeConfirmDialog({ target, busy, onClose, onConfirm, error }: ConfirmProps) {
  return (
    <Dialog open={target !== null} onClose={onClose} title="Delete permanently?">
      <div className="space-y-3">
        <p className="text-sm leading-relaxed text-ink-dim">
          {target && (
            <>
              <span className="font-medium text-ink">{target.item.title}</span>{" "}
              <span className="font-mono text-xs">({target.item.branch})</span> will be removed
              from disk along with the local branch. This cannot be undone.
            </>
          )}
        </p>
        {error && isAppError(error) ? <ErrorNote error={error} /> : null}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="button" variant="danger" busy={busy} onClick={onConfirm}>
            Delete permanently
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
