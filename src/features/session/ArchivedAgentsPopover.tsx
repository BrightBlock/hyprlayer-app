import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type Ref,
  type RefObject,
} from "react";

import { claudeModels } from "@/lib/domain/models";
import { formatCompactAge } from "@/lib/format";
import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { useClaudeProbe } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { ConfirmDialog, cx, Popover, ROW_ACTION, ROW_ACTION_DANGER } from "@/components/ui";

import {
  archivedModelLabel,
  deleteArchivedAgent,
  newestFirst,
  restoreArchivedAgent,
} from "./archivedAgents";
import { useWorkstreamTabsStore, type ArchivedShell, type ArchivedStatus } from "./workstreamTabs";

// The window header, the caret gap, this panel's header and filter row, and margin.
const CHROME_PX = 40 + 10 + 38 + 38 + 8;
const LIST_MAX_HEIGHT = `calc(100dvh - ${CHROME_PX + STATUS_BAR_HEIGHT}px)`;

const NONE: ArchivedShell[] = [];

const STATUS: Record<ArchivedStatus, { label: string; dot: string }> = {
  done: { label: "Done", dot: "bg-ok" },
  stopped: { label: "Stopped", dot: "bg-warn" },
  idle: { label: "Idle", dot: "bg-ink-4" },
};

type Row = { entry: ArchivedShell; model: string };

/** Once `gone` has left the list, focus returns to the row at `index`. */
type FocusAfter = { index: number; gone: number[] };

export function ArchivedAgentsPopover({
  workstreamId,
  open,
  onClose,
  anchorRef,
}: {
  workstreamId: string;
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
      side="top"
      align="start"
      sideOffset={10}
      caret
      trapFocus
      aria-labelledby={headingId}
      className="w-[440px] max-w-[calc(100vw-16px)] p-0 shadow-pop"
    >
      <PopoverBody workstreamId={workstreamId} headingId={headingId} onClose={onClose} />
    </Popover>
  );
}

function PopoverBody({
  workstreamId,
  headingId,
  onClose,
}: {
  workstreamId: string;
  headingId: string;
  onClose: () => void;
}) {
  const archived = useWorkstreamTabsStore((s) => s.byWs[workstreamId]?.archived) ?? NONE;
  const probed = useClaudeProbe().data?.models;
  const catalog = useMemo(() => claudeModels(probed), [probed]);

  const searchId = useId();
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<{ entry: ArchivedShell; index: number } | null>(null);
  const [active, setActive] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const rowRefs = useRef<(HTMLLIElement | null)[]>([]);
  const focusAfter = useRef<FocusAfter | null>(null);

  const rows = useMemo<Row[]>(
    () => newestFirst(archived).map((entry) => ({ entry, model: archivedModelLabel(entry.shell, catalog) })),
    [archived, catalog],
  );
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter(
      (row) => row.entry.shell.title.toLowerCase().includes(q) || row.model.toLowerCase().includes(q),
    );
  }, [rows, query]);
  const current = Math.min(active, Math.max(visible.length - 1, 0));

  // Runs once the dialog is gone and `inert` has lifted, so focus can land.
  useEffect(() => {
    const target = focusAfter.current;
    if (pending || !target) return;
    if (target.gone.some((id) => visible.some((row) => row.entry.shell.id === id))) return;
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

  const closePending = (gone: number[] = []) => {
    focusAfter.current = { index: current, gone };
    setPending(null);
  };

  const restore = (entry: ArchivedShell) => {
    onClose();
    restoreArchivedAgent(workstreamId, entry.shell.id);
  };

  const askDelete = (entry: ArchivedShell, index: number) => {
    setActive(index);
    setPending({ entry, index });
  };

  const onRowKey = (entry: ArchivedShell, index: number) => (e: KeyboardEvent<HTMLLIElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "ArrowDown") focusRow(index + 1);
    else if (e.key === "ArrowUp") {
      if (index === 0) searchRef.current?.focus();
      else focusRow(index - 1);
    } else if (e.key === "Home") focusRow(0);
    else if (e.key === "End") focusRow(visible.length - 1);
    else if (e.key === "Enter") restore(entry);
    else if (e.key === "Delete" || e.key === "Backspace") askDelete(entry, index);
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
    searchRef.current?.focus();
  };

  const now = Date.now();
  return (
    <>
      <div inert={pending !== null}>
        <div className="flex items-baseline justify-between gap-3 px-3 pb-2.5 pt-3">
          <span id={headingId} className="lbl">
            Archived agents
          </span>
          {rows.length > 0 && <span className="text-meta tabular-nums text-ink-dim">{rows.length}</span>}
        </div>

        <div className="flex items-center gap-2 px-3 pb-2.5">
          <div className="relative min-w-0 flex-1">
            <label htmlFor={searchId} className="sr-only">
              Filter archived agents
            </label>
            <Icon
              name="search"
              size="sm"
              className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-ink-dim"
            />
            <input
              id={searchId}
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onSearchKey}
              placeholder="Filter by name or model"
              spellCheck={false}
              autoComplete="off"
              className={cx(
                "h-7 w-full rounded-control border border-edge bg-surface pl-7 pr-2 text-ui text-ink",
                "placeholder:text-ink-dim focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
              )}
            />
          </div>
        </div>

        {rows.length === 0 ? (
          <p className="border-t border-edge-2 px-3 py-6 text-center text-ui text-ink-dim">
            No archived agents.
          </p>
        ) : visible.length === 0 ? (
          <p className="border-t border-edge-2 px-3 py-6 text-center text-ui text-ink-dim">
            <span>{`No matches for "${query.trim()}"`}</span>{" "}
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
              <ArchivedAgentRow
                key={row.entry.shell.id}
                row={row}
                now={now}
                focusable={index === current}
                rowRef={(el) => {
                  rowRefs.current[index] = el;
                }}
                onFocus={() => setActive(index)}
                onKeyDown={onRowKey(row.entry, index)}
                onRestore={() => restore(row.entry)}
                onDelete={() => askDelete(row.entry, index)}
              />
            ))}
          </ul>
        )}
      </div>

      {pending && (
        <ConfirmDialog
          open
          onClose={() => closePending()}
          title={`Delete ${pending.entry.shell.title}?`}
          confirmLabel="Delete"
          confirmVariant="danger"
          autoFocus="confirm"
          onConfirm={() => {
            deleteArchivedAgent(workstreamId, pending.entry.shell.id);
            closePending([pending.entry.shell.id]);
          }}
        >
          <p className="text-body text-ink-2">
            It leaves this workstream's archive and can't be restored. Its branch and worktree belong to
            the workstream and stay put.
          </p>
        </ConfirmDialog>
      )}
    </>
  );
}

function ArchivedAgentRow({
  row,
  now,
  focusable,
  rowRef,
  onFocus,
  onKeyDown,
  onRestore,
  onDelete,
}: {
  row: Row;
  now: number;
  focusable: boolean;
  rowRef: Ref<HTMLLIElement>;
  onFocus: () => void;
  onKeyDown: (e: KeyboardEvent<HTMLLIElement>) => void;
  onRestore: () => void;
  onDelete: () => void;
}) {
  const { entry, model } = row;
  const { title } = entry.shell;
  const status = STATUS[entry.status];
  const meta = [model, formatCompactAge(entry.archivedAt, now)].filter(Boolean).join(" · ");
  return (
    <li
      ref={rowRef}
      tabIndex={focusable ? 0 : -1}
      aria-label={`${title}, ${status.label}, ${meta}`}
      onFocus={onFocus}
      onKeyDown={onKeyDown}
      className={cx(
        "group flex items-center gap-[11px] border-t border-edge-2 px-3.5 py-2.5",
        "hover:bg-panel-2 focus-within:bg-panel-2 focus:outline-none",
        "focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60",
      )}
    >
      <span aria-hidden="true" className={cx("h-[7px] w-[7px] shrink-0 rounded-full", status.dot)} />
      <div className="min-w-0 flex-1">
        <div
          className="truncate text-meta font-medium text-ink group-hover:font-semibold group-focus-within:font-semibold"
          title={title}
        >
          {title}
        </div>
        <div className="mt-0.5 truncate text-meta tabular-nums text-ink-dim">{meta}</div>
      </div>
      <span className="hidden shrink-0 items-center gap-1.5 group-hover:flex group-focus-within:flex">
        <button type="button" onClick={onRestore} aria-label={`Restore ${title}`} className={ROW_ACTION}>
          Restore
        </button>
        <button type="button" onClick={onDelete} aria-label={`Delete ${title}`} className={ROW_ACTION_DANGER}>
          Delete
        </button>
      </span>
    </li>
  );
}
