import type { WorkstreamView } from "@/lib/bindings";
import { WorkstreamActivityDot } from "@/features/session/WorkstreamActivity";
import { WorkstreamGitChip } from "@/features/workstreams/WorkstreamGitChip";
import { Badge } from "@/components/ui";
import { isDetachedHead, isFolderWorkstream, isOffBase, shortBranch } from "@/lib/domain/projects";

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
  // In-place workstreams have no dedicated worktree — they run in the project's
  // own checkout. Flag that so the branch pill isn't mistaken for a worktree.
  const inplace = item.worktreeMode === "inplace";
  // A folder workstream has no branch to pill at all.
  const folder = isFolderWorkstream(view);
  const detached = isDetachedHead(view);
  // The checkout is on a branch other than its merge base. Only flagged for
  // in-place rows (a worktree-backed one is off its base by construction), and
  // never while detached — the stored branch is stale then, so it would
  // contradict the "detached HEAD" chip beside it.
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
