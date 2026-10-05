import { ActivityDot } from "@/components/ActivityDot";
import { rowClass } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { formatCompactAge, formatRelativeTime } from "@/lib/format";
import {
  cardEvents,
  harnessLabel,
  useAwaitingSince,
  workstreamPlace,
  type PipelineCardHandlers,
} from "./PipelineCard";
import { TRIAGE_GROUPS, triageKeyOf, type PipelineLive, type PipelineSession, type TriageKey } from "./pipeline";

export function PipelineTriage({
  views,
  live,
  sessions,
  now,
  handlers,
}: {
  views: WorkstreamView[];
  live: Map<string, PipelineLive>;
  sessions: Map<string, PipelineSession>;
  now: number;
  handlers: PipelineCardHandlers;
}) {
  const grouped = new Map<TriageKey, WorkstreamView[]>(TRIAGE_GROUPS.map((g) => [g.key, []]));
  for (const view of views) grouped.get(triageKeyOf(live.get(view.item.id)))?.push(view);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-3 pb-5">
      {TRIAGE_GROUPS.filter((g) => (grouped.get(g.key)?.length ?? 0) > 0).map((group) => (
        <section key={group.key} aria-label={group.label} className="mb-5">
          <div className="mb-1.5 flex items-center gap-2 px-2">
            <span className={`h-2 w-2 shrink-0 rounded-full ${group.dot}`} />
            <h2 className="text-ui font-semibold text-ink">{group.label}</h2>
            <span className="font-mono text-meta text-ink-4">{grouped.get(group.key)?.length}</span>
          </div>
          <div className="flex flex-col">
            {grouped.get(group.key)?.map((view) => (
              <TriageRow
                key={view.item.id}
                view={view}
                live={live.get(view.item.id)}
                session={sessions.get(view.item.id)}
                now={now}
                handlers={handlers}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

function TriageRow({
  view,
  live,
  session,
  now,
  handlers,
}: {
  view: WorkstreamView;
  live?: PipelineLive;
  session?: PipelineSession;
  now: number;
  handlers: PipelineCardHandlers;
}) {
  const since = useAwaitingSince(live);
  const waiting = live?.activity === "needs-you";
  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={view.item.title}
      {...cardEvents(view, live, handlers)}
      className={rowClass({ tier: "standard", className: "flex gap-3 px-2" })}
    >
      <span className="flex w-2 shrink-0 justify-center">
        {live ? <ActivityDot activity={live.activity} /> : <span className="h-1.5 w-1.5 rounded-full bg-ink-dim/40" />}
      </span>
      <span className="w-64 shrink-0 truncate text-body font-medium text-ink">{view.item.title}</span>
      <span className="w-56 shrink-0 truncate font-mono text-meta text-ink-4">
        {[workstreamPlace(view), harnessLabel(session)].filter(Boolean).join(" · ")}
      </span>
      <span className="min-w-0 flex-1 truncate text-ui-sm text-ink-dim">{live?.line}</span>
      {waiting && (
        <span className="shrink-0 rounded-control bg-q-chip-bg px-1.5 py-0.5 font-mono text-meta font-semibold text-q-chip-text">
          {since !== null ? `Waited ${formatCompactAge(since, now)}` : "Waiting"}
        </span>
      )}
      <span className="w-20 shrink-0 text-right font-mono text-meta text-ink-4">
        {waiting ? (
          <span className="font-sans font-semibold text-accent">Answer</span>
        ) : (
          formatRelativeTime(view.item.updatedAt, now)
        )}
      </span>
    </div>
  );
}
