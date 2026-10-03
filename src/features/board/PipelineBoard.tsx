import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { MERGED_VISIBLE, PIPELINE_COLUMNS, pipelineColumnOf, type PipelineKey, type PipelineWaiting } from "./pipeline";
import { PipelineCard } from "./PipelineCard";

const byUpdated = (a: WorkstreamView, b: WorkstreamView) => (b.item.updatedAt ?? 0) - (a.item.updatedAt ?? 0);

export function PipelineBoard({
  views,
  waiting,
  merged,
  projectName,
  now,
  onOpen,
  onOpenArchived,
}: {
  views: WorkstreamView[];
  waiting: Map<string, PipelineWaiting>;
  merged: ArchivedWorkstreamView[];
  projectName: (path: string) => string;
  now: number;
  onOpen: (view: WorkstreamView, sessionId: string | null) => void;
  onOpenArchived: () => void;
}) {
  const grouped: Record<Exclude<PipelineKey, "merged">, WorkstreamView[]> = {
    planning: [],
    implementing: [],
    waiting: [],
    review: [],
  };
  for (const view of [...views].sort(byUpdated)) {
    grouped[pipelineColumnOf(view, waiting.has(view.item.id))].push(view);
  }
  const branches = views.filter((v) => !isFolderWorkstream(v)).length;
  const hiddenMerged = merged.length - MERGED_VISIBLE;

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface">
      <header className="px-6 pt-[18px] pb-2">
        <h1 className="text-h2 text-ink">Pipeline</h1>
      </header>
      <div className="flex min-h-0 flex-1 gap-3.5 overflow-hidden px-6 pt-3 pb-5">
        {PIPELINE_COLUMNS.map((col) => {
          const count = col.key === "merged" ? merged.length : grouped[col.key].length;
          return (
            <section key={col.key} className="flex min-w-0 flex-1 flex-col">
              <div className="mb-2.5 flex items-center gap-2">
                <span className={`h-2 w-2 shrink-0 rounded-full ${col.dot}`} />
                <h2 className="text-ui font-semibold text-ink">{col.label}</h2>
                <span className={`font-mono text-meta ${col.count}`}>{count}</span>
              </div>
              <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto">
                {col.key === "merged" ? (
                  <>
                    {merged.slice(0, MERGED_VISIBLE).map((v) => (
                      <button
                        key={v.item.id}
                        type="button"
                        onClick={onOpenArchived}
                        className="block w-full rounded-card border border-edge bg-panel px-3 py-[11px] text-left opacity-80 shadow-sm transition-colors hover:border-ink-dim/50"
                      >
                        <span className="block truncate text-body font-semibold text-ink">{v.item.title}</span>
                        <span className="mt-1 block truncate font-mono text-meta text-ink-4">
                          {projectName(v.item.projectPath)}
                        </span>
                      </button>
                    ))}
                    {hiddenMerged > 0 && (
                      <p className="text-center text-meta text-ink-4">+{hiddenMerged} more today</p>
                    )}
                  </>
                ) : (
                  grouped[col.key].map((view) => (
                    <PipelineCard
                      key={view.item.id}
                      view={view}
                      projectName={projectName(view.item.projectPath)}
                      waiting={waiting.get(view.item.id)}
                      now={now}
                      onOpen={onOpen}
                    />
                  ))
                )}
              </div>
            </section>
          );
        })}
      </div>
      <p className="px-6 pb-3 font-mono text-meta text-ink-dim">
        {`${branches} ${branches === 1 ? "branch" : "branches"} active`}
      </p>
    </div>
  );
}
