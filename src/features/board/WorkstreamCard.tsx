import type { WorkstreamView } from "@/lib/bindings";
import { WorkstreamActivityDot } from "@/features/session/WorkstreamActivity";
import { WorkstreamGitChip } from "@/features/workstreams/WorkstreamGitChip";
import { Badge } from "@/components/ui";
import { isDetachedHead, isFolderWorkstream, isOffBase, shortBranch } from "@/lib/domain/projects";

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
  const inplace = item.worktreeMode === "inplace";
  const folder = isFolderWorkstream(view);
  const detached = isDetachedHead(view);
  const branchedInPlace = inplace && !detached && isOffBase(view);
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
      title={
        folder
          ? `${item.title} · project folder`
          : `${item.title} · ${detached ? "detached HEAD" : item.branch}${
              inplace ? ` · main checkout (base: ${shortBranch(item.baseBranch)})` : ""
            }`
      }
    >
      <div className="flex items-start gap-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">{item.title}</span>
        <WorkstreamActivityDot workstreamId={item.id} />
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        {folder ? (
          <Badge tone="dim">folder</Badge>
        ) : (
          <>
            <span className="max-w-full truncate rounded bg-edge/60 px-1.5 py-0.5 font-mono text-xs text-ink-dim">
              {detached ? "detached HEAD" : item.branch}
            </span>
            {inplace && <Badge tone="dim">no worktree</Badge>}
            {branchedInPlace && <Badge tone="dim">off base</Badge>}
            <WorkstreamGitChip view={view} />
            {item.prNumber != null && <Badge tone="accent">PR #{item.prNumber}</Badge>}
          </>
        )}
      </div>
    </div>
  );
}
