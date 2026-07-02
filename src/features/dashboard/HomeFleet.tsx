/**
 * The home dashboard's fleet surfaces: the needs-you banner, the Live-now
 * strip (one slim row per workstream with agent activity), and the Recent
 * workstreams card (quiet actives + freshly archived ones). Pure presentation —
 * classification lives in `fleet.ts`, navigation in the parent's `onOpen`.
 */
import { useState, type ReactNode } from "react";

import type { ArchivedWorkstreamView, WorkstreamView } from "../../lib/bindings";
import { formatElapsed, formatRelativeTime, formatTokens } from "../session/toolDisplay";
import type { FleetActivity, FleetItem } from "./fleet";

/** Coarse elapsed for the strip ("7m", "1h 04m") — minute-level so the rows
 *  don't visibly tick between the dashboard's slow clock beats. */
function shortElapsed(ms: number): string {
  if (ms < 60_000) return "<1m";
  const min = Math.floor(ms / 60_000);
  if (min < 60) return `${min}m`;
  return formatElapsed(ms);
}

const DOT: Record<FleetActivity, string> = {
  working: "bg-accent animate-pulse",
  background: "bg-background animate-pulse",
  "needs-you": "bg-attention animate-pulse",
  review: "bg-ok",
  error: "bg-bad",
};

function Dot({ activity }: { activity: FleetActivity }) {
  return <span className={`h-[7px] w-[7px] shrink-0 rounded-full ${DOT[activity]}`} />;
}

function Pill({ tone, children }: { tone: string; children: ReactNode }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center rounded-full px-2 py-px text-[10px] font-bold ${tone}`}
    >
      {children}
    </span>
  );
}

// ── Needs-you banner ─────────────────────────────────────────────────────────

/** The single most urgent blocked agent, promoted above the fold. */
export function NeedsYouBanner({ item, onOpen }: { item: FleetItem; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex w-full items-center gap-3 rounded-xl border border-attention/40 bg-attention/10 px-4 py-3 text-left hover:bg-attention/15"
    >
      <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-attention" />
      <span className="shrink-0 text-[13px] font-bold text-attention">Needs you</span>
      <span className="shrink-0 truncate text-[13px] text-ink">
        <b>{item.view.item.title}</b> is waiting on your input
      </span>
      {item.line && (
        <span className="hidden min-w-0 truncate rounded-md bg-attention/15 px-2 py-0.5 font-mono text-[11px] text-ink md:block">
          {item.line}
        </span>
      )}
      <span className="min-w-0 flex-1" />
      <span className="shrink-0 rounded-lg bg-attention px-3.5 py-1.5 text-xs font-bold text-surface">
        Review →
      </span>
    </button>
  );
}

// ── Live-now strip ───────────────────────────────────────────────────────────

function LiveRow({
  item,
  now,
  onOpen,
}: {
  item: FleetItem;
  now: number;
  onOpen: (item: FleetItem) => void;
}) {
  const { activity, view, line, meter, since, queued } = item;
  const git = view.git;
  const running = activity === "working" || activity === "background";
  const context =
    activity === "review"
      ? git.dirty > 0
        ? null // rendered as colored diff stats below
        : "turn finished — ready for review"
      : line;
  return (
    <button
      type="button"
      onClick={() => onOpen(item)}
      className="flex w-full items-center gap-2.5 px-3.5 py-2.5 text-left text-[12.5px] hover:bg-panel-2/60"
    >
      <Dot activity={activity} />
      <span
        className={`w-[190px] shrink-0 truncate font-bold ${activity === "error" ? "text-ink-dim" : "text-ink"}`}
      >
        {view.item.title}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-dim">
        {activity === "review" && git.dirty > 0 ? (
          <>
            <span className="text-ok">+{git.additions}</span>{" "}
            <span className="text-bad">−{git.deletions}</span> · {git.dirty}{" "}
            {git.dirty === 1 ? "file" : "files"}
          </>
        ) : (
          <>
            {context}
            {queued > 0 && ` · +${queued} queued`}
          </>
        )}
      </span>
      {running && meter && (
        <>
          <span className="relative h-[3px] w-[90px] shrink-0 overflow-hidden rounded-full bg-panel-2">
            <span
              className="absolute inset-y-0 left-0 rounded-full bg-accent"
              style={{ width: `${Math.max(2, Math.round(meter.fraction * 100))}%` }}
            />
          </span>
          <span className="w-10 shrink-0 text-right font-mono text-[10px] text-ink-dim">
            {formatTokens(meter.tokens)}
          </span>
        </>
      )}
      {running && since !== null && (
        <span className="w-9 shrink-0 text-right font-mono text-[10.5px] text-accent">
          {shortElapsed(now - since)}
        </span>
      )}
      {activity === "background" && <Pill tone="bg-background/15 text-background">TASKS</Pill>}
      {activity === "needs-you" && <Pill tone="bg-attention/15 text-attention">NEEDS YOU</Pill>}
      {activity === "error" && <Pill tone="bg-bad/15 text-bad">ERROR</Pill>}
      {activity === "review" && (
        <>
          <Pill tone="bg-ok/15 text-ok">REVIEW</Pill>
          <span className="shrink-0 text-[11.5px] font-semibold text-accent">
            {view.item.prNumber !== null ? "Open PR →" : "Open →"}
          </span>
        </>
      )}
    </button>
  );
}

/** Rows shown before the strip caps and scrolls — a busy fleet otherwise grows
 *  the card unbounded and buries whatever sits below it on the page. */
const LIVE_CAP = 6;

/** One slim row per live workstream — every agent state glanceable at once.
 *  Past LIVE_CAP rows the strip caps to a scroll region, with a toggle to
 *  expand it to its full height (and collapse back). */
export function LiveNowCard({
  items,
  now,
  onOpen,
  emptyHint,
}: {
  items: FleetItem[];
  now: number;
  onOpen: (item: FleetItem) => void;
  emptyHint: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const capped = items.length > LIVE_CAP && !expanded;

  return (
    <div className="overflow-hidden rounded-xl border border-edge bg-panel [view-transition-name:live-now]">
      {items.length === 0 ? (
        <div className="flex items-center gap-2.5 px-3.5 py-2.5 text-[12.5px] text-ink-dim">
          <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-edge" />
          {emptyHint}
        </div>
      ) : (
        <>
          <div className={`divide-y divide-panel-2 ${capped ? "max-h-64 overflow-y-auto" : ""}`}>
            {items.map((item) => (
              <LiveRow key={item.view.item.id} item={item} now={now} onOpen={onOpen} />
            ))}
          </div>
          {items.length > LIVE_CAP && (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="flex w-full items-center justify-center gap-1 border-t border-panel-2 py-2 text-[11.5px] font-semibold text-accent hover:bg-panel-2/60"
            >
              {expanded ? "Show less ↑" : `Show all ${items.length} ↓`}
            </button>
          )}
        </>
      )}
    </div>
  );
}

// ── Recent workstreams ───────────────────────────────────────────────────────

export type RecentRow =
  | { kind: "active"; at: number; view: WorkstreamView; project: string }
  | { kind: "archived"; at: number; view: ArchivedWorkstreamView; project: string; merged: boolean };

/** Quiet actives and freshly archived workstreams, newest first — the "what
 *  was I doing" list under the live strip. */
export function RecentWorkstreams({
  rows,
  now,
  onOpenActive,
  onOpenArchive,
}: {
  rows: RecentRow[];
  now: number;
  onOpenActive: (view: WorkstreamView) => void;
  onOpenArchive: () => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-edge bg-panel [view-transition-name:recent-ws]">
      <div className="flex items-center px-4 pb-2 pt-3">
        <span className="text-[12.5px] font-bold text-ink">Recent workstreams</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onOpenArchive}
          className="text-[11.5px] text-ink-dim hover:text-ink"
        >
          View all
        </button>
      </div>
      {rows.length === 0 && (
        <p className="border-t border-panel-2 px-4 py-2.5 text-[12.5px] text-ink-dim">
          Nothing here yet — workstreams you finish or set aside land here.
        </p>
      )}
      {rows.map((row) => (
        <button
          key={`${row.kind}-${row.view.item.id}`}
          type="button"
          onClick={() => (row.kind === "active" ? onOpenActive(row.view) : onOpenArchive())}
          className="flex w-full items-center gap-2.5 border-t border-panel-2 px-4 py-2 text-left text-[12.5px] hover:bg-panel-2/60"
        >
          <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-edge" />
          <span className="min-w-0 truncate font-semibold text-ink">{row.view.item.title}</span>
          <span className="shrink-0 font-mono text-[10.5px] text-ink-dim">{row.project}</span>
          <span className="min-w-0 flex-1" />
          {row.kind === "archived" && (
            <span
              className={`shrink-0 font-mono text-[10.5px] ${row.merged ? "text-ok" : "text-ink-dim"}`}
            >
              {row.merged ? "merged" : "archived"}
            </span>
          )}
          <span className="shrink-0 text-[11px] text-ink-dim">
            {formatRelativeTime(row.at, now).replace(" ago", "")}
          </span>
        </button>
      ))}
    </div>
  );
}
