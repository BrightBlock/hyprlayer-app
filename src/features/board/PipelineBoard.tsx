import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";
import { shortBranch } from "@/lib/domain/projects";
import { PipelineCard, type PipelineCardHandlers } from "./PipelineCard";
import {
  MERGED_VISIBLE,
  PIPELINE_COLUMNS,
  pipelineColumnOf,
  type PipelineKey,
  type PipelineLive,
  type PipelineSession,
} from "./pipeline";

export function PipelineBoard({
  views,
  live,
  sessions,
  merged,
  now,
  handlers,
  onOpenArchived,
}: {
  views: WorkstreamView[];
  live: Map<string, PipelineLive>;
  sessions: Map<string, PipelineSession>;
  merged: ArchivedWorkstreamView[];
  now: number;
  handlers: PipelineCardHandlers;
  onOpenArchived: () => void;
}) {
  const grouped: Record<Exclude<PipelineKey, "merged">, WorkstreamView[]> = {
    planning: [],
    implementing: [],
    waiting: [],
    review: [],
  };
  for (const view of views) {
    grouped[pipelineColumnOf(view, live.get(view.item.id)?.activity === "needs-you")].push(view);
  }
  const hiddenMerged = merged.length - MERGED_VISIBLE;

  return (
    <div className="flex min-h-0 flex-1 gap-3.5 overflow-x-auto px-6 pt-3 pb-5">
      {PIPELINE_COLUMNS.map((col) => {
        const count = col.key === "merged" ? merged.length : grouped[col.key].length;
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
              {col.key === "merged" ? (
                <>
                  {merged.slice(0, MERGED_VISIBLE).map((v) => (
                    <div key={v.item.id} role="listitem">
                      <button
                        type="button"
                        onClick={onOpenArchived}
                        className="block w-full rounded-card border border-edge bg-panel px-3 py-[11px] text-left opacity-80 shadow-sm transition-colors hover:border-ink-dim/50"
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
              ) : (
                grouped[col.key].map((view) => (
                  <div key={view.item.id} role="listitem">
                    <PipelineCard
                      view={view}
                      live={live.get(view.item.id)}
                      session={sessions.get(view.item.id)}
                      now={now}
                      handlers={handlers}
                    />
                  </div>
                ))
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}

