import { useEffect, useRef, useState } from "react";

import { Icon } from "@/components/Icon";
import { Badge, Button, ChordHint } from "@/components/ui";
import { closeReview } from "@/lib/appNav";
import type { ReviewFileList, ReviewView, SubmitReviewResult, WorkstreamView } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { basename } from "@/lib/path";
import { IS_MACOS } from "@/lib/platform";
import { knownPr } from "@/lib/domain/pr";
import { useBootstrap, useWorkstreamPr } from "@/lib/queries";
import { isInlinePlanKey } from "@/features/session/planPresentation";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

import { clearSubmitRequest, useSubmitRequested } from "./reviewStore";
import { SubmitReviewPopover } from "./SubmitReviewPopover";

function Sep() {
  return <span className="text-ink-4">·</span>;
}

function StatusChip({
  review,
  undelivered,
  prNumber,
  agentLabel,
  agentWorking,
  delivering,
  onResend,
}: {
  review: ReviewView | undefined;
  undelivered: boolean;
  prNumber: number | null;
  agentLabel: string;
  agentWorking: boolean;
  delivering: boolean;
  onResend: () => void;
}) {
  if (undelivered) {
    return (
      <button
        type="button"
        disabled={delivering}
        onClick={onResend}
        className="shrink-0 rounded-control disabled:opacity-60"
      >
        <Badge tone="bad">Not delivered · Resend</Badge>
      </button>
    );
  }
  if (prNumber !== null) return <Badge tone="ok">Shipped · PR #{prNumber}</Badge>;
  if (review?.status === "changes_requested") {
    return (
      <Badge tone="warn">{agentWorking ? `Changes requested · ${agentLabel} is fixing` : "Changes requested"}</Badge>
    );
  }
  if (review?.status === "approved") return <Badge tone="ok">Approved</Badge>;
  return null;
}

export function ReviewHeader({
  view,
  review,
  files,
  viewedCount,
  markingAll,
  onMarkAllViewed,
  planKey,
  agentLabel,
  agentWorking,
  agentBusy,
  delivering,
  onResend,
  onRequested,
  onOpenDoc,
}: {
  view: WorkstreamView;
  review: ReviewView | undefined;
  files: ReviewFileList | undefined;
  viewedCount: number;
  markingAll: boolean;
  onMarkAllViewed: () => void;
  planKey: string | null;
  agentLabel: string;
  agentWorking: boolean;
  agentBusy: boolean;
  delivering: boolean;
  onResend: () => void;
  onRequested: (result: SubmitReviewResult) => void;
  onOpenDoc: (path: string) => void;
}) {
  const git = useWorkstreamGitActions(view);
  const submitRef = useRef<HTMLButtonElement>(null);
  const [submitting, setSubmitting] = useState(false);
  const submitRequested = useSubmitRequested(view.item.id);
  const pr = knownPr(useWorkstreamPr(view.item.id).data);
  const prNumber = pr && (pr.state === "open" || pr.state === "merged") ? pr.number : null;
  const unsent = review?.comments.filter((c) => c.state === "pending" || c.state === "saved").length ?? 0;
  const latest = review?.rounds[review.rounds.length - 1];
  // Only a change request is ever delivered; other verdicts stay `delivered: false`.
  const undelivered = latest?.verdict === "request_changes" && !latest.delivered;
  const loaded = review !== undefined;
  useEffect(() => {
    if (!submitRequested || !loaded) return;
    clearSubmitRequest();
    if (!undelivered) setSubmitting(true);
  }, [submitRequested, loaded, undelivered]);
  const overrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const shipChord = effectiveChord(overrides, "ws.ship", IS_MACOS);
  const total = files?.files.length ?? 0;
  const pct = total > 0 ? (viewedCount / total) * 100 : 0;
  const planDoc = planKey && !isInlinePlanKey(planKey) ? planKey : null;

  return (
    <div className="shrink-0 border-b border-edge bg-panel px-4 py-2.5">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={closeReview}
          className="flex shrink-0 items-center gap-0.5 text-ui font-medium text-ink-dim hover:text-ink"
        >
          <Icon name="chevronLeft" size="sm" />
          Chat
        </button>
        <h1 className="min-w-0 truncate text-title text-ink">Review changes</h1>
        <StatusChip
          review={review}
          undelivered={undelivered}
          prNumber={prNumber}
          agentLabel={agentLabel}
          agentWorking={agentWorking}
          delivering={delivering}
          onResend={onResend}
        />
        <span className="flex-1" />
        {total > 0 && (
          <span className="flex shrink-0 items-center gap-2">
            {viewedCount < total && (
              <Button size="sm" variant="ghost" busy={markingAll} onClick={onMarkAllViewed}>
                Mark all viewed
              </Button>
            )}
            <span className="text-meta tabular-nums text-ink-dim">
              {viewedCount} / {total} viewed
            </span>
            <span
              role="progressbar"
              aria-label="Files viewed"
              aria-valuemin={0}
              aria-valuemax={total}
              aria-valuenow={viewedCount}
              className="h-1 w-28 overflow-hidden rounded-[3px] bg-panel-2"
            >
              <span className="block h-full rounded-[3px] bg-ink" style={{ width: `${pct}%` }} />
            </span>
          </span>
        )}
        <Button
          ref={submitRef}
          size="sm"
          disabled={!review || undelivered}
          aria-expanded={submitting}
          onClick={() => setSubmitting(true)}
        >
          Submit review
          {unsent > 0 && <Badge tone="dim">{unsent}</Badge>}
        </Button>
        {review && (
          <SubmitReviewPopover
            open={submitting}
            onClose={() => setSubmitting(false)}
            anchorRef={submitRef}
            workstreamId={view.item.id}
            unsent={unsent}
            viewedCount={viewedCount}
            total={total}
            agentLabel={agentLabel}
            agentBusy={agentBusy}
            onRequested={onRequested}
          />
        )}
        {!git.hasLivePr && (
          <span title={git.implementWorking ? `Ship unlocks when ${agentLabel} finishes` : undefined}>
            <Button size="sm" variant="primary" disabled={!git.canShip} busy={git.shipping} onClick={git.doShip}>
              <Icon name="ship" size="sm" />
              Ship
              {shipChord && <ChordHint chord={shipChord} className="text-meta opacity-70" />}
            </Button>
          </span>
        )}
      </div>
      <div className="mt-1 flex min-w-0 items-center gap-2 text-meta text-ink-dim">
        {planDoc && (
          <>
            <button
              type="button"
              onClick={() => onOpenDoc(planDoc)}
              className="min-w-0 truncate text-ink-2 hover:text-ink hover:underline"
              title={planDoc}
            >
              {basename(planDoc).replace(/\.md$/, "")}
            </button>
            <Sep />
          </>
        )}
        <span className="min-w-0 truncate font-mono">
          {view.item.branch} → {files?.base ?? view.item.baseBranch}
        </span>
        {files && !files.baseResolved && (
          <>
            <Sep />
            <span className="shrink-0 text-warn-ink">base not found · diffing against HEAD</span>
          </>
        )}
        {files && (
          <>
            <Sep />
            <span className="shrink-0 tabular-nums">
              {plural(total, "file")} <span className="text-diff-add">+{files.additions}</span>{" "}
              <span className="text-diff-del">−{files.deletions}</span>
            </span>
          </>
        )}
      </div>
    </div>
  );
}
