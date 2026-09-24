import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { Search } from "lucide-react";

import { formatBytes } from "@/lib/format";
import { isAppError } from "@/lib/ipc";
import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { basename } from "@/lib/path";
import {
  useArchivedWorkstreams,
  useBootstrap,
  useDeleteArchivedWorkstream,
  useKeepArchivedWorkstream,
  usePurgeArchivedWorkstreams,
  useRefreshArchivedWorkstreams,
} from "@/lib/queries";
import { ICON } from "@/components/lucide";
import { ConfirmDialog, cx, ErrorNote, Popover } from "@/components/ui";

import { useRestoreAndOpen } from "./archiveActions";
import { ArchivedWorktreeRow, ROW_ACTION } from "./ArchivedWorktreeRow";
import {
  filterArchived,
  lossOnDelete,
  mergedPurgeSelection,
  toArchiveRow,
  totalBytes,
  type ArchiveRow,
  type DeleteLoss,
} from "./archiveStatus";
import { RestoreTargetDialog } from "./RestoreTargetDialog";

// The window header, the caret gap, and this panel's own header, filter row,
// footer and margin: the list scrolls within what is left.
const CHROME_PX = 40 + 10 + 38 + 38 + 43 + 8;
const LIST_MAX_HEIGHT = `calc(100dvh - ${CHROME_PX + STATUS_BAR_HEIGHT}px)`;

type Pending =
  | { kind: "delete"; row: ArchiveRow; index: number }
  | { kind: "purge"; rows: ArchiveRow[] }
  | { kind: "restore-to"; row: ArchiveRow };

/** Once `gone` has left the list, focus returns to the row at `index`. */
type FocusAfter = { index: number; gone: string[] };

export function ArchivedWorktreesPopover({
  open,
  onClose,
  anchorRef,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
}) {
  const headingId = useId();
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      align="end"
      sideOffset={10}
      caret
      trapFocus
      aria-labelledby={headingId}
      className="w-[440px] max-w-[calc(100vw-16px)] p-0 shadow-[0_12px_30px_rgba(0,0,0,0.18)]"
    >
      <PopoverBody headingId={headingId} onClose={onClose} />
    </Popover>
  );
}

function PopoverBody({ headingId, onClose }: { headingId: string; onClose: () => void }) {
  const archived = useArchivedWorkstreams();
  const projects = useBootstrap().data?.projects;
  const { mutate: refresh } = useRefreshArchivedWorkstreams();
  const keep = useKeepArchivedWorkstream();
  const remove = useDeleteArchivedWorkstream();
  const purge = usePurgeArchivedWorkstreams();
  const restore = useRestoreAndOpen(onClose);

  const searchId = useId();
  const [query, setQuery] = useState("");
  const [mergedOnly, setMergedOnly] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [active, setActive] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const rowRefs = useRef<(HTMLLIElement | null)[]>([]);
  const focusAfter = useRef<FocusAfter | null>(null);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const rows = useMemo(() => {
    const names = new Map((projects ?? []).map((p) => [p.path, p.mappedName]));
    return (archived.data ?? []).map((view) =>
      toArchiveRow(view, names.get(view.item.projectPath) ?? basename(view.item.projectPath)),
    );
  }, [archived.data, projects]);
  const visible = useMemo(() => filterArchived(rows, query, mergedOnly), [rows, query, mergedOnly]);
  const purgeable = useMemo(() => mergedPurgeSelection(rows), [rows]);
  const current = Math.min(active, Math.max(visible.length - 1, 0));

  // Runs once the dialog is gone and `inert` has lifted, so focus can land.
  useEffect(() => {
    const target = focusAfter.current;
    if (pending || !target) return;
    if (target.gone.some((id) => visible.some((row) => row.id === id))) return;
    focusAfter.current = null;
    if (visible.length === 0) {
      searchRef.current?.focus();
      return;
    }
    const index = Math.min(target.index, visible.length - 1);
    setActive(index);
    rowRefs.current[index]?.focus();
  }, [pending, visible]);

  const focusRow = (index: number) => {
    const next = Math.max(0, Math.min(index, visible.length - 1));
    setActive(next);
    rowRefs.current[next]?.focus();
  };

  const closePending = (gone: string[] = []) => {
    focusAfter.current = { index: current, gone };
    setPending(null);
  };

  const startRestore = (row: ArchiveRow, target: string | null = null) => {
    if (!row.view.archiveExists || restore.pending) return;
    restore.run(row.id, target).catch((e: unknown) => {
      if (target !== null) return; // shown in the restore-to dialog
      if (isAppError(e) && e.code === "restore_target_occupied") {
        restore.reset();
        setPending({ kind: "restore-to", row });
      } else {
        setError(e);
      }
    });
  };

  const askDelete = (row: ArchiveRow, index: number) => {
    remove.reset();
    setActive(index);
    setPending({ kind: "delete", row, index });
  };

  const onRowKey = (row: ArchiveRow, index: number) => (e: KeyboardEvent<HTMLLIElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "ArrowDown") focusRow(index + 1);
    else if (e.key === "ArrowUp") {
      if (index === 0) searchRef.current?.focus();
      else focusRow(index - 1);
    } else if (e.key === "Home") focusRow(0);
    else if (e.key === "End") focusRow(visible.length - 1);
    else if (e.key === "Enter") startRestore(row);
    else if (e.key === "Delete" || e.key === "Backspace") askDelete(row, index);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  const onSearchKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "ArrowDown" || visible.length === 0) return;
    e.preventDefault();
    focusRow(current);
  };

  const clearFilter = () => {
    setQuery("");
    setMergedOnly(false);
    searchRef.current?.focus();
  };

  const now = Date.now();
  return (
    <>
      <div inert={pending !== null}>
        <div className="flex items-baseline justify-between gap-3 px-3 pb-2.5 pt-3">
          <span id={headingId} className="lbl">
            Archived worktrees
          </span>
          {rows.length > 0 && (
            <span className="font-mono text-[10.5px] tabular-nums text-ink-dim">
              {`${rows.length} · ${formatBytes(totalBytes(rows))}`}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2 px-3 pb-2.5">
          <div className="relative min-w-0 flex-1">
            <label htmlFor={searchId} className="sr-only">
              Filter archived worktrees
            </label>
            <Search
              {...ICON}
              size={14}
              className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-ink-dim"
            />
            <input
              id={searchId}
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onSearchKey}
              placeholder="Filter by branch or project"
              spellCheck={false}
              autoComplete="off"
              className={cx(
                "h-7 w-full rounded-control border border-edge bg-surface pl-7 pr-2 text-xs text-ink",
                "placeholder:text-ink-dim focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
              )}
            />
          </div>
          <button
            type="button"
            aria-pressed={mergedOnly}
            onClick={() => setMergedOnly((on) => !on)}
            className={cx(ROW_ACTION, "h-7 px-2.5", mergedOnly && "bg-accent-soft text-ink hover:bg-accent-soft")}
          >
            Merged
          </button>
        </div>

        {(error != null || archived.error != null) && (
          <div className="px-3 pb-2.5">
            <ErrorNote error={error ?? archived.error} onDismiss={() => setError(null)} />
          </div>
        )}

        {archived.data === undefined ? null : rows.length === 0 ? (
          <p className="border-t border-edge-2 px-3 py-6 text-center text-xs text-ink-dim">
            No archived worktrees.
          </p>
        ) : visible.length === 0 ? (
          <p className="border-t border-edge-2 px-3 py-6 text-center text-xs text-ink-dim">
            <span>{query.trim() ? `No matches for "${query.trim()}"` : "No merged worktrees."}</span>{" "}
            <button
              type="button"
              onClick={clearFilter}
              className="text-ink underline-offset-2 hover:underline focus:outline-none focus-visible:underline"
            >
              Clear
            </button>
          </p>
        ) : (
          <ul aria-labelledby={headingId} className="overflow-y-auto" style={{ maxHeight: LIST_MAX_HEIGHT }}>
            {visible.map((row, index) => (
              <ArchivedWorktreeRow
                key={row.id}
                row={row}
                now={now}
                focusable={index === current}
                busy={restore.pending}
                rowRef={(el) => {
                  rowRefs.current[index] = el;
                }}
                onFocus={() => setActive(index)}
                onKeyDown={onRowKey(row, index)}
                onRestore={() => startRestore(row)}
                onDelete={() => askDelete(row, index)}
                onKeep={() => keep.mutate(row.id, { onError: setError })}
              />
            ))}
          </ul>
        )}

        {rows.length > 0 && (
          <div className="flex justify-end border-t border-edge px-3 py-[9px]">
            <button
              type="button"
              disabled={purgeable.length === 0}
              onClick={() => {
                purge.reset();
                setPending({ kind: "purge", rows: purgeable });
              }}
              className={ROW_ACTION}
            >
              Purge merged
            </button>
          </div>
        )}
      </div>

      {pending?.kind === "delete" && (
        <DeleteConfirm
          row={pending.row}
          busy={remove.isPending}
          error={remove.error}
          onDismissError={remove.reset}
          onClose={() => closePending()}
          onConfirm={() =>
            remove.mutate(pending.row.id, {
              onSuccess: () => closePending([pending.row.id]),
            })
          }
        />
      )}
      {pending?.kind === "purge" && (
        <PurgeConfirm
          rows={pending.rows}
          busy={purge.isPending}
          error={purge.error}
          onDismissError={purge.reset}
          onClose={() => closePending()}
          onConfirm={() => {
            const ids = pending.rows.map((row) => row.id);
            purge.mutate(ids, {
              onSuccess: (report) => {
                if (report.failed > 0) {
                  setError(new Error(`${report.failed} of ${ids.length} could not be deleted.`));
                }
                closePending(ids);
              },
            });
          }}
        />
      )}
      {pending?.kind === "restore-to" && (
        <RestoreTargetDialog
          row={pending.row}
          busy={restore.pending}
          error={restore.error}
          onDismissError={restore.reset}
          onClose={() => {
            restore.reset();
            closePending();
          }}
          onRestore={(target) => startRestore(pending.row, target)}
        />
      )}
    </>
  );
}

function DeleteConfirm({
  row,
  busy,
  error,
  onDismissError,
  onClose,
  onConfirm,
}: {
  row: ArchiveRow;
  busy: boolean;
  error: unknown;
  onDismissError: () => void;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const loss = lossOnDelete(row);
  const { item, info, archiveExists } = row.view;
  const ownFiles = item.worktreeMode === "isolated" || item.worktreeMode === "existing";
  const size = info.sizeBytes === null ? "" : ` (${formatBytes(info.sizeBytes)})`;
  const what = !ownFiles
    ? "It has no worktree of its own, so this removes only the record and its agent history."
    : !archiveExists
      ? "Its files are already gone, so this removes the record and its agent history."
      : `Deletes the archived worktree${size}${info.deletesBranch ? " and its branch" : ""}, with its agent history.`;
  return (
    <ConfirmDialog
      open
      onClose={onClose}
      title={`Delete ${row.branch}?`}
      confirmLabel={loss ? "Delete anyway" : "Delete"}
      confirmVariant="danger"
      autoFocus={loss ? "cancel" : "confirm"}
      busy={busy}
      error={error}
      onDismissError={onDismissError}
      onConfirm={onConfirm}
    >
      <p className="text-sm text-ink-2">{what}</p>
      {loss && <p className="text-sm text-bad-ink">{lossText(loss)}</p>}
    </ConfirmDialog>
  );
}

function lossText(loss: DeleteLoss): string {
  if (loss.unchecked) {
    return "Its files couldn't be checked for uncommitted work, so anything there would be lost.";
  }
  const parts: string[] = [];
  if (loss.uncommitted > 0) {
    parts.push(`${loss.uncommitted} ${loss.uncommitted === 1 ? "file" : "files"} with uncommitted changes`);
  }
  if (loss.unpushed > 0) {
    parts.push(`${loss.unpushed} unpushed ${loss.unpushed === 1 ? "commit" : "commits"}`);
  }
  return `It has ${parts.join(" and ")}, which will be lost.`;
}

function PurgeConfirm({
  rows,
  busy,
  error,
  onDismissError,
  onClose,
  onConfirm,
}: {
  rows: ArchiveRow[];
  busy: boolean;
  error: unknown;
  onDismissError: () => void;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const count = rows.length;
  const dirty = rows.filter((row) => (row.view.info.uncommitted ?? 0) > 0).length;
  return (
    <ConfirmDialog
      open
      onClose={onClose}
      title={`Delete ${count} merged ${count === 1 ? "worktree" : "worktrees"} (${formatBytes(totalBytes(rows))})?`}
      confirmLabel={dirty > 0 ? "Delete anyway" : "Delete"}
      confirmVariant="danger"
      autoFocus={dirty > 0 ? "cancel" : "confirm"}
      busy={busy}
      error={error}
      onDismissError={onDismissError}
      onConfirm={onConfirm}
    >
      <p className="text-sm text-ink-2">Their work is merged. Kept archives are left alone.</p>
      {dirty > 0 && (
        <p className="text-sm text-bad-ink">
          {dirty === 1 ? "1 of them has" : `${dirty} of them have`} uncommitted changes, which will
          be lost.
        </p>
      )}
    </ConfirmDialog>
  );
}
