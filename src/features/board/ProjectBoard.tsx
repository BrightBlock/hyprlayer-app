import type { WorkstreamView } from "@/lib/bindings";
import { Button } from "@/components/ui";
import { PlusIcon } from "@/components/icons";
import { BOARD_COLUMNS, groupByColumn } from "./board";
import { BoardColumn } from "./BoardColumn";
import { WorkstreamCard } from "./WorkstreamCard";

/** The project-level kanban board. Shows active workstreams bucketed into
 *  pipeline-stage lanes. The board has no header of its own — the top bar's
 *  breadcrumb already names it and carries the "+" new-workstream button.
 *  Empty projects show a "create your first workstream" CTA. */
export function ProjectBoard({
  views,
  onOpen,
  onNewWorkstream,
  onCardContextMenu,
}: {
  views: WorkstreamView[];
  onOpen: (id: string) => void;
  onNewWorkstream: () => void;
  onCardContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
}) {
  const grouped = groupByColumn(views);

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface">
      {views.length === 0 ? (
        /* Empty-project state: same copy as the workstream chat empty state. */
        <div className="min-h-0 flex-1 overflow-y-auto p-6">
          <div className="mx-auto max-w-3xl space-y-4">
            <div className="rounded-lg border border-edge bg-panel p-6 text-center">
              <p className="text-base font-medium text-ink">Create your first workstream</p>
              <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-ink-dim">
                Each workstream gets its own branch and worktree, so agents can research, plan, and
                implement without touching your checkout.
              </p>
              <Button variant="primary" className="mt-4" onClick={onNewWorkstream}>
                <PlusIcon />
                New workstream
              </Button>
            </div>
          </div>
        </div>
      ) : (
        /* Grid layout: the 6 stage tiles fill the whole canvas — 3 columns × 2
           rows on a wide window (dropping to 2 columns on narrower ones), each
           row an equal fraction of the height (auto-rows-fr) so tiles fill the
           vertical space instead of leaving it blank. A busy tile scrolls
           internally rather than stretching the grid. */
        <div className="grid min-h-0 flex-1 auto-rows-fr grid-cols-2 gap-8 px-6 py-6 lg:grid-cols-3">
          {BOARD_COLUMNS.map((col) => (
            <BoardColumn
              key={col.key}
              label={col.label}
              stageKey={col.stageKey}
              count={grouped[col.key].length}
            >
              {grouped[col.key].map((view) => (
                <div key={view.item.id} role="listitem">
                  <WorkstreamCard view={view} onOpen={onOpen} onContextMenu={onCardContextMenu} />
                </div>
              ))}
            </BoardColumn>
          ))}
        </div>
      )}
    </div>
  );
}
