import { Icon } from "@/components/Icon";
import { Button, ChordHint } from "@/components/ui";
import { openReview } from "@/lib/appNav";
import type { WorkstreamView } from "@/lib/bindings";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { findWorkstreamForNotify } from "@/lib/notifications";
import { IS_MACOS } from "@/lib/platform";
import { useBootstrap, useReview, useWorkstreams } from "@/lib/queries";
import { usePlanTracker } from "@/features/plan/usePlanTracker";
import type { ReadyCardProps } from "@/features/session/readyCardSlot";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

export function ReadyForReviewCard({ sessionId, workstreamId }: ReadyCardProps) {
  const projectPath = findWorkstreamForNotify(workstreamId)?.projectPath;
  return projectPath ? (
    <ReadyCard sessionId={sessionId} workstreamId={workstreamId} projectPath={projectPath} />
  ) : null;
}

function ReadyCard({ sessionId, workstreamId, projectPath }: ReadyCardProps & { projectPath: string }) {
  const view = useWorkstreams(projectPath).data?.find((w) => w.item.id === workstreamId);
  return view ? <ReadyCardBody sessionId={sessionId} view={view} /> : null;
}

function ReadyCardBody({ sessionId, view }: { sessionId: string; view: WorkstreamView }) {
  const tracker = usePlanTracker(sessionId).tracker;
  const git = useWorkstreamGitActions(view);
  const tasksDone = tracker?.tasksDone ?? false;
  // get_review creates the review row, writes the exclude file and ingests notes.
  const review = useReview(view.item.id, tasksDone && !git.implementWorking).data;
  const overrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const reviewChord = effectiveChord(overrides, "ws.review", IS_MACOS);
  const shipChord = effectiveChord(overrides, "ws.ship", IS_MACOS);

  const files = review?.files;
  if (!tracker || !tasksDone || git.implementWorking || !review || !files || files.files.length === 0) return null;
  if (review.status === "approved") return null;
  const calls = review.notes.filter((n) => n.kind === "needs_your_call" && !n.resolved).length;

  return (
    <div className="mt-4 max-w-[min(520px,100%)] space-y-2.5 rounded-card border border-edge bg-panel px-3 py-2.5">
      <div className="flex items-center gap-1.5">
        <Icon name="review" size="sm" className="shrink-0 text-accent" />
        <span className="text-ui font-semibold text-ink">Ready for review</span>
        <span className="flex-1" />
        <span className="shrink-0 text-meta tabular-nums">
          <span className="text-diff-add">+{files.additions}</span>{" "}
          <span className="text-diff-del">−{files.deletions}</span>
        </span>
      </div>
      <div className="min-w-0 space-y-0.5">
        <div className="truncate text-body text-ink">{tracker.title}</div>
        <div className="truncate font-mono text-meta text-ink-dim">
          {view.item.branch} → {files.base}
        </div>
      </div>
      <dl className="flex gap-5 text-ui">
        <div className="flex items-baseline gap-1.5">
          <dt className="text-ink-dim">Files</dt>
          <dd className="tabular-nums text-ink">{files.files.length} changed</dd>
        </div>
        <div className="flex items-baseline gap-1.5">
          <dt className="text-ink-dim">Notes</dt>
          <dd className="tabular-nums text-ink">
            {review.notes.length} · {calls} {calls === 1 ? "needs" : "need"} your call
          </dd>
        </div>
      </dl>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="primary"
          onClick={() => openReview({ projectPath: view.item.projectPath, workstreamId: view.item.id })}
        >
          Review changes
          {reviewChord && <ChordHint chord={reviewChord} className="text-meta opacity-70" />}
        </Button>
        <Button size="sm" disabled={!git.canShip} busy={git.shipping} onClick={git.doShip}>
          <Icon name="ship" size="sm" />
          Ship
          {shipChord && <ChordHint chord={shipChord} className="text-meta opacity-70" />}
        </Button>
      </div>
    </div>
  );
}
