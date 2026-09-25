import type { WorkstreamView } from "@/lib/bindings";
import { Button } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { BOARD_COLUMNS, groupByColumn } from "./board";
import { BoardColumn } from "./BoardColumn";
import { WorkstreamCard } from "./WorkstreamCard";

export function ProjectBoard({
  views,
  folder = false,
  onOpen,
  onNewWorkstream,
  onCardContextMenu,
}: {
  views: WorkstreamView[];
  folder?: boolean;
  onOpen: (id: string) => void;
  onNewWorkstream: () => void;
  onCardContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
}) {
  const grouped = groupByColumn(views);

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface">
      {views.length === 0 ? (
        <div className="min-h-0 flex-1 overflow-y-auto p-6">
          <div className="mx-auto max-w-3xl space-y-4">
            <div className="rounded-lg border border-edge bg-panel p-6 text-center">
              <p className="text-title text-ink">Create your first workstream</p>
              <p className="mx-auto mt-1 max-w-md text-body leading-prose text-ink-dim">
                {folder
                  ? "Each workstream is its own conversation, all running in this folder. Start as many as you need."
                  : "Each workstream gets its own branch and worktree, so agents can research, plan, and implement without touching your checkout."}
              </p>
              <Button variant="primary" className="mt-4" onClick={onNewWorkstream}>
                <Icon name="add" />
                New workstream
              </Button>
            </div>
          </div>
        </div>
      ) : (
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
