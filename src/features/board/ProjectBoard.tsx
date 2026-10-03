import type { WorkstreamView } from "@/lib/bindings";
import { BOARD_COLUMNS, groupByColumn } from "./board";
import { BoardColumn } from "./BoardColumn";
import { NoWorktreeBlank } from "./NoWorktreeBlank";
import { WorkstreamCard } from "./WorkstreamCard";

export function ProjectBoard({
  projectPath,
  views,
  folder = false,
  onOpen,
  onNewWorkstream,
  onRestore,
  onCardContextMenu,
}: {
  projectPath: string;
  views: WorkstreamView[];
  folder?: boolean;
  onOpen: (id: string) => void;
  onNewWorkstream: () => void;
  onRestore: () => void;
  onCardContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
}) {
  const grouped = groupByColumn(views);

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface">
      {views.length === 0 ? (
        <NoWorktreeBlank projectPath={projectPath} folder={folder} onNew={onNewWorkstream} onRestore={onRestore} />
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
