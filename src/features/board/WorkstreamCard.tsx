import type { WorkstreamView } from "../../lib/bindings";
import { WorkstreamActivityDot } from "../session/WorkstreamActivity";
import { WorkstreamGitChip } from "../workstreams/WorkstreamGitChip";
import { Badge } from "../../components/ui";

/** A clickable kanban card representing one active workstream. Clicking (or
 *  pressing Enter / Space) calls `onOpen` with the workstream's id; right-click
 *  opens the card's context menu via `onContextMenu` (e.g. Attach PR). */
export function WorkstreamCard({
  view,
  onOpen,
  onContextMenu,
}: {
  view: WorkstreamView;
  onOpen: (id: string) => void;
  onContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
}) {
  const { item } = view;
  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={item.title}
      onClick={() => onOpen(item.id)}
      onContextMenu={
        onContextMenu &&
        ((e) => {
          e.preventDefault();
          onContextMenu(view, e.clientX, e.clientY);
        })
      }
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen(item.id);
        }
      }}
      className="group cursor-pointer rounded-lg border border-edge bg-panel-2 p-3 text-left transition-colors hover:border-ink-dim/50"
      title={`${item.title} · ${item.branch}`}
    >
      <div className="flex items-start gap-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">{item.title}</span>
        <WorkstreamActivityDot workstreamId={item.id} />
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <span className="max-w-full truncate rounded bg-edge/60 px-1.5 py-0.5 font-mono text-xs text-ink-dim">
          {item.branch}
        </span>
        <WorkstreamGitChip view={view} />
        {item.prNumber != null && <Badge tone="accent">PR #{item.prNumber}</Badge>}
      </div>
    </div>
  );
}
