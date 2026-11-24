import type { KeyboardEvent, Ref } from "react";

import { formatBytes, formatCompactAge } from "@/lib/format";
import { cx, ROW_ACTION, ROW_ACTION_DANGER } from "@/components/ui";

import type { ArchiveRow } from "./archiveStatus";
import { STATUS_DOT, StatusTag } from "./StatusTag";

type Props = {
  row: ArchiveRow;
  now: number;
  focusable: boolean;
  busy: boolean;
  rowRef: Ref<HTMLLIElement>;
  onFocus: () => void;
  onKeyDown: (e: KeyboardEvent<HTMLLIElement>) => void;
  onRestore: () => void;
  onDelete: () => void;
  onKeep: () => void;
};

function metaLine(row: ArchiveRow, now: number): string {
  const { item, info, archiveExists } = row.view;
  const size = !archiveExists
    ? "files missing"
    : info.sizeBytes === null
      ? null
      : formatBytes(info.sizeBytes);
  return [
    row.project,
    row.status.prRef,
    row.status.uncommittedWork ? "uncommitted work" : null,
    formatCompactAge(item.archivedAt, now),
    size,
  ]
    .filter(Boolean)
    .join(" · ");
}

export function ArchivedWorktreeRow({
  row,
  now,
  focusable,
  busy,
  rowRef,
  onFocus,
  onKeyDown,
  onRestore,
  onDelete,
  onKeep,
}: Props) {
  const { status, branch } = row;
  const meta = metaLine(row, now);
  return (
    <li
      ref={rowRef}
      tabIndex={focusable ? 0 : -1}
      aria-label={[branch, status.tag, meta].filter(Boolean).join(", ")}
      onFocus={onFocus}
      onKeyDown={onKeyDown}
      className={cx(
        "group flex items-center gap-[11px] border-t border-edge-2 px-3.5 py-2.5",
        "hover:bg-panel-2 focus-within:bg-panel-2 focus:outline-none",
        status.purgePending && "opacity-75",
      )}
    >
      <span aria-hidden="true" className={cx("h-[7px] w-[7px] shrink-0 rounded-full", STATUS_DOT[status.tone])} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-meta font-medium text-ink group-hover:font-semibold group-focus-within:font-semibold">
            {branch}
          </span>
          {status.tag && <StatusTag tone={status.tone} label={status.tag} />}
        </div>
        <div className="mt-0.5 truncate text-meta tabular-nums text-ink-dim">{meta}</div>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <span className="hidden items-center gap-1.5 group-hover:flex group-focus-within:flex">
          <button
            type="button"
            onClick={onRestore}
            disabled={busy || !row.view.archiveExists}
            aria-label={`Restore ${branch}`}
            title={row.view.archiveExists ? undefined : "Its archived files are missing"}
            className={ROW_ACTION}
          >
            Restore
          </button>
          <button
            type="button"
            onClick={onDelete}
            disabled={busy}
            aria-label={`Delete ${branch}`}
            className={ROW_ACTION_DANGER}
          >
            Delete
          </button>
        </span>
        {status.purgePending && (
          <button
            type="button"
            onClick={onKeep}
            disabled={busy}
            aria-label={`Keep ${branch}`}
            className={ROW_ACTION}
          >
            Keep
          </button>
        )}
      </div>
    </li>
  );
}
