import { useId, useState, type ReactNode } from "react";

import { ActivityDot } from "@/components/ActivityDot";
import { Icon } from "@/components/Icon";
import { cx, rowClass } from "@/components/ui";
import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";
import { shortBranch } from "@/lib/domain/projects";
import type { LeadAsk } from "@/features/lead/lead";
import { cardEvents, PipelineCard, type PipelineCardHandlers } from "./PipelineCard";
import {
  boardColumns,
  MERGED_VISIBLE,
  PIPELINE_COLUMNS,
  pipelineColumnOf,
  type BoardChild,
  type PipelineLive,
  type PipelineSession,
} from "./pipeline";

export function PipelineBoard({
  views,
  live,
  sessions,
  merged,
  driven,
  leadCard,
  ask,
  now,
  handlers,
  onOpenArchived,
}: {
  views: WorkstreamView[];
  live: Map<string, PipelineLive>;
  sessions: Map<string, PipelineSession>;
  merged: ArchivedWorkstreamView[];
  driven: ReadonlySet<string>;
  leadCard: ReactNode;
  ask: LeadAsk | null;
  now: number;
  handlers: PipelineCardHandlers;
  onOpenArchived: () => void;
}) {
  const askFor = (id: string) => (ask?.subject?.view.item.id === id ? ask : undefined);
  // A child the Lead asks about keeps its own card, so its Archive / Later strip shows even beside its parent.
  const grouped = boardColumns(
    views,
    (v) => pipelineColumnOf(v, askFor(v.item.id) !== undefined || live.get(v.item.id)?.activity === "needs-you"),
    (v) => askFor(v.item.id) !== undefined,
  );
  const hiddenMerged = merged.length - MERGED_VISIBLE;

  return (
    <div className="flex min-h-0 flex-1 gap-3.5 overflow-x-auto px-6 pt-3 pb-5">
      {PIPELINE_COLUMNS.map((col) => {
        const extra = col.key === "merged" ? merged.length : col.key === "waiting" && leadCard ? 1 : 0;
        const count = grouped[col.key].length + extra;
        return (
          <section key={col.key} aria-label={col.label} className="flex min-w-[200px] flex-1 flex-col">
            <div className="mb-2.5 flex items-center gap-2 px-1">
              <span
                className={`h-2 w-2 shrink-0 rounded-full ${col.dot} ${
                  col.key === "implementing" && count > 0 ? "ring-[3px] ring-accent/15" : ""
                }`}
              />
              <h2 className="flex-1 text-ui font-semibold text-ink">{col.label}</h2>
              <span className={`font-mono text-meta ${col.count}`}>{count}</span>
            </div>
            <div role="list" className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto">
              {col.key === "waiting" && leadCard && <div role="listitem">{leadCard}</div>}
              {grouped[col.key].map(({ view, parent, children }) => (
                <div key={view.item.id} role="listitem">
                  <PipelineCard
                    view={view}
                    live={live.get(view.item.id)}
                    session={sessions.get(view.item.id)}
                    driven={driven.has(view.item.id)}
                    ask={askFor(view.item.id)}
                    now={now}
                    handlers={handlers}
                    parentTitle={parent?.item.title}
                    fold={children.length > 0 && <SubWorkstreams items={children} live={live} handlers={handlers} />}
                  />
                </div>
              ))}
              {col.key === "merged" && (
                <>
                  {merged.slice(0, MERGED_VISIBLE).map((v) => (
                    <div key={v.item.id} role="listitem">
                      <button
                        type="button"
                        onClick={onOpenArchived}
                        className="block w-full rounded-card border border-edge bg-panel px-3 py-[11px] text-left opacity-80 shadow-sm hover:border-ink-dim/50"
                      >
                        <span className="block truncate text-body font-semibold text-ink">{v.item.title}</span>
                        <span className="mt-1 block truncate font-mono text-meta text-ink-4">
                          {shortBranch(v.item.branch)}
                        </span>
                      </button>
                    </div>
                  ))}
                  {hiddenMerged > 0 && (
                    <button
                      type="button"
                      onClick={onOpenArchived}
                      className="py-1.5 text-center font-mono text-meta text-ink-4 hover:text-ink"
                    >
                      +{hiddenMerged} more today
                    </button>
                  )}
                </>
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}


function SubWorkstreams({
  items,
  live,
  handlers,
}: {
  items: BoardChild[];
  live: Map<string, PipelineLive>;
  handlers: PipelineCardHandlers;
}) {
  const [open, setOpen] = useState(false);
  const listId = useId();

  return (
    <div className="px-3 pb-[11px]">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={listId}
        onClick={() => setOpen((o) => !o)}
        className="-ml-1 flex items-center gap-1.5 rounded px-1 font-mono text-meta text-ink-dim hover:text-ink"
      >
        <span className={cx("transition-transform", open && "rotate-90")}>
          <Icon name="chevronRight" size="xs" />
        </span>
        {items.length === 1 ? "1 sub-workstream" : `${items.length} sub-workstreams`}
      </button>
      {open && (
        <ul id={listId} className="mt-1 ml-1 border-l border-edge pl-1.5">
          {items.map(({ view, column }) => {
            const childLive = live.get(view.item.id);
            return (
              <li key={view.item.id}>
                <button
                  type="button"
                  {...cardEvents(view, childLive, handlers)}
                  className={rowClass({ tier: "compact", className: "flex w-full gap-2 px-1.5 text-left" })}
                >
                  <ActivityDot activity={childLive?.activity ?? "idle"} idle="dot" />
                  <span className="min-w-0 flex-1 truncate text-ui-sm text-ink-2">{view.item.title}</span>
                  <span className="shrink-0 font-mono text-meta text-ink-4">
                    {PIPELINE_COLUMNS.find((c) => c.key === column)?.label}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
