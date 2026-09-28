import {
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type Ref,
  type RefObject,
} from "react";

import type { Session } from "@/lib/bindings";
import { harnessOption } from "@/lib/domain/harnessing";
import { stageLabel } from "@/lib/domain/stages";
import { formatCompactAge } from "@/lib/format";
import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { useActiveSessions, useWorkstreamSessions } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { cx, Popover } from "@/components/ui";

import { openSessionTab, useWorkstreamTabsStore, type ShellTab } from "./workstreamTabs";

// The window header, the caret gap, this panel's header and filter row, and margin.
const CHROME_PX = 40 + 10 + 38 + 38 + 8;
const LIST_MAX_HEIGHT = `calc(100dvh - ${CHROME_PX + STATUS_BAR_HEIGHT}px)`;

const NONE_SESSIONS: Session[] = [];
const NONE_SHELLS: ShellTab[] = [];

type Row = { session: Session; title: string; live: boolean; localOpen: boolean };

export function AgentSessionsPopover({
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
  const sessions = useWorkstreamSessions(workstreamId).data ?? NONE_SESSIONS;
  const activeSessions = useActiveSessions().data;
  const liveIds = useMemo(
    () => new Set((activeSessions ?? []).map((a) => a.session.id)),
    [activeSessions],
  );
  const localShells = useWorkstreamTabsStore((s) => s.byWs[workstreamId]?.shells) ?? NONE_SHELLS;
  const localTitles = useMemo(() => {
    const map = new Map<string, string>();
    for (const shell of localShells) if (shell.sessionId) map.set(shell.sessionId, shell.title);
    return map;
  }, [localShells]);

  const searchId = useId();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const rowRefs = useRef<(HTMLLIElement | null)[]>([]);

  const rows = useMemo<Row[]>(
    () =>
      [...sessions]
        .sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0))
        .map((session) => ({
          session,
          title: localTitles.get(session.id) ?? session.title ?? session.preview ?? stageLabel(session.stage),
          live: liveIds.has(session.id),
          localOpen: localTitles.has(session.id),
        })),
    [sessions, localTitles, liveIds],
  );
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter(
      (row) =>
        row.title.toLowerCase().includes(q) ||
        (row.session.preview?.toLowerCase().includes(q) ?? false) ||
        stageLabel(row.session.stage).toLowerCase().includes(q),
    );
  }, [rows, query]);
  const current = Math.min(active, Math.max(visible.length - 1, 0));

  const focusRow = (index: number) => {
    const next = Math.max(0, Math.min(index, visible.length - 1));
    setActive(next);
    rowRefs.current[next]?.focus();
  };

  const openRow = (session: Session) => {
    onClose();
    openSessionTab(workstreamId, session);
  };

  const onRowKey = (session: Session, index: number) => (e: KeyboardEvent<HTMLLIElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "ArrowDown") focusRow(index + 1);
    else if (e.key === "ArrowUp") {
      if (index === 0) searchRef.current?.focus();
      else focusRow(index - 1);
    } else if (e.key === "Home") focusRow(0);
    else if (e.key === "End") focusRow(visible.length - 1);
    else if (e.key === "Enter") openRow(session);
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
      <div className="flex items-baseline justify-between gap-3 px-3 pb-2.5 pt-3">
        <span id={headingId} className="lbl">
          Agent sessions
        </span>
        {rows.length > 0 && <span className="text-meta tabular-nums text-ink-dim">{rows.length}</span>}
      </div>

      <div className="flex items-center gap-2 px-3 pb-2.5">
        <div className="relative min-w-0 flex-1">
          <label htmlFor={searchId} className="sr-only">
            Filter agent sessions
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
            placeholder="Filter by title or prompt"
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
          No agent sessions.
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
            <AgentSessionRow
              key={row.session.id}
              row={row}
              now={now}
              focusable={index === current}
              rowRef={(el) => {
                rowRefs.current[index] = el;
              }}
              onFocus={() => setActive(index)}
              onKeyDown={onRowKey(row.session, index)}
              onOpen={() => openRow(row.session)}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function AgentSessionRow({
  row,
  now,
  focusable,
  rowRef,
  onFocus,
  onKeyDown,
  onOpen,
}: {
  row: Row;
  now: number;
  focusable: boolean;
  rowRef: Ref<HTMLLIElement>;
  onFocus: () => void;
  onKeyDown: (e: KeyboardEvent<HTMLLIElement>) => void;
  onOpen: () => void;
}) {
  const { session, title, live, localOpen } = row;
  const meta = [
    stageLabel(session.stage),
    harnessOption(session.harness).label,
    formatCompactAge(session.startedAt, now),
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <li
      ref={rowRef}
      tabIndex={focusable ? 0 : -1}
      aria-label={[title, live ? "Live" : null, meta].filter(Boolean).join(", ")}
      onFocus={onFocus}
      onKeyDown={onKeyDown}
      onClick={onOpen}
      className={cx(
        "group flex cursor-pointer items-center gap-[11px] border-t border-edge-2 px-3.5 py-2.5",
        "hover:bg-panel-2 focus-within:bg-panel-2 focus:outline-none",
        "focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60",
      )}
    >
      <span
        aria-hidden="true"
        className={cx("h-[7px] w-[7px] shrink-0 rounded-full", live ? "bg-ok" : "bg-transparent")}
      />
      <div className="min-w-0 flex-1">
        <div
          className="truncate text-meta font-medium text-ink group-hover:font-semibold group-focus-within:font-semibold"
          title={title}
        >
          {title}
        </div>
        <div className="mt-0.5 truncate text-meta tabular-nums text-ink-dim">{meta}</div>
      </div>
      {localOpen && <span className="shrink-0 text-meta text-ink-dim">Open</span>}
    </li>
  );
}
