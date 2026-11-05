import { ActivityDot } from "@/components/ActivityDot";
import type { WorkstreamView } from "@/lib/bindings";
import { stageLabel } from "@/lib/domain/stages";
import { formatCompactCount, formatRelativeTime } from "@/lib/format";
import { cardEvents, harnessLabel, workstreamPlace, type PipelineCardHandlers } from "./PipelineCard";
import {
  PIPELINE_COLUMNS,
  columnDot,
  pipelineColumnOf,
  prNumberOf,
  type PipelineLive,
  type PipelineSession,
} from "./pipeline";

const HEAD = "px-2 py-2 text-left font-mono text-label text-ink-4";
const CELL = "px-2 py-2 align-middle";

export function PipelineTable({
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
  return (
    <div className="min-h-0 flex-1 overflow-auto px-6 pt-3 pb-5">
      <table className="w-full min-w-[880px] border-collapse">
        <thead className="sticky top-0 bg-surface">
          <tr className="border-b border-edge">
            <th className={HEAD}>Workstream</th>
            <th className={HEAD}>Branch</th>
            <th className={HEAD}>Pipeline</th>
            <th className={HEAD}>Stage</th>
            <th className={HEAD}>Activity</th>
            <th className={HEAD}>Agent</th>
            <th className={`${HEAD} text-right`}>Diff</th>
            <th className={HEAD}>PR</th>
            <th className={`${HEAD} text-right`}>Updated</th>
          </tr>
        </thead>
        <tbody>
          {views.map((view) => {
            const l = live.get(view.item.id);
            const s = sessions.get(view.item.id);
            const column = pipelineColumnOf(view, l?.activity === "needs-you");
            const { additions, deletions } = view.git;
            const pr = prNumberOf(view);
            return (
              <tr
                key={view.item.id}
                role="button"
                tabIndex={0}
                aria-label={view.item.title}
                {...cardEvents(view, l, handlers)}
                className="cursor-pointer border-b border-edge/60 hover:bg-edge/40 focus:outline-none focus-visible:bg-edge/40"
              >
                <td className={`${CELL} max-w-64 truncate text-body font-medium text-ink`}>{view.item.title}</td>
                <td className={`${CELL} max-w-48 truncate font-mono text-meta text-ink-dim`}>{workstreamPlace(view)}</td>
                <td className={CELL}>
                  <span className="flex items-center gap-1.5 text-ui text-ink-2">
                    <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${columnDot(column)}`} />
                    {PIPELINE_COLUMNS.find((c) => c.key === column)?.label}
                  </span>
                </td>
                <td className={`${CELL} text-ui text-ink-dim`}>{stageLabel(view.item.stage)}</td>
                <td className={`${CELL} max-w-56`}>
                  <span className="flex min-w-0 items-center gap-1.5">
                    {l && <ActivityDot activity={l.activity} />}
                    <span className="truncate text-ui-sm text-ink-dim">{l?.line ?? (l ? "" : "idle")}</span>
                  </span>
                </td>
                <td className={`${CELL} whitespace-nowrap font-mono text-meta text-ink-dim`}>
                  {[harnessLabel(s), s?.model].filter(Boolean).join(" · ")}
                </td>
                <td className={`${CELL} whitespace-nowrap text-right font-mono text-meta`}>
                  {additions > 0 && <span className="text-diff-add">+{formatCompactCount(additions)} </span>}
                  {deletions > 0 && <span className="text-diff-del">−{formatCompactCount(deletions)}</span>}
                </td>
                <td className={`${CELL} whitespace-nowrap font-mono text-meta text-ink-dim`}>
                  {pr != null ? `#${pr}` : ""}
                </td>
                <td className={`${CELL} whitespace-nowrap text-right font-mono text-meta text-ink-4`}>
                  {formatRelativeTime(view.item.updatedAt, now)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
