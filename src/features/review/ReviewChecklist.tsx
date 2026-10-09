import { useMemo } from "react";

import { Icon } from "@/components/Icon";
import { Badge, Button, rowClass } from "@/components/ui";
import { openReview } from "@/lib/appNav";
import type { ReviewView } from "@/lib/bindings";
import { harnessLabel } from "@/lib/domain/harnessing";
import { plural } from "@/lib/format";
import { basename } from "@/lib/path";
import { useReview, useWorkstreamSessions } from "@/lib/queries";
import type { ReviewChecklistProps } from "@/features/plan/reviewChecklistSlot";
import { newestImplementSession, useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

import { useReviewDelivery } from "./useReviewDelivery";

function summarize(review: ReviewView) {
  const files = review.files.files;
  const viewed = new Set(review.viewed);
  const viewedCount = files.filter((f) => viewed.has(f.path)).length;
  const requests = review.rounds.filter((r) => r.verdict === "request_changes");
  const delivered = requests.filter((r) => r.delivered);
  const lastSent = delivered[delivered.length - 1] ?? null;
  const latest = review.rounds[review.rounds.length - 1] ?? null;
  const touched = new Set(review.changedSinceRound);
  const changed = files.filter((f) => touched.has(f.path));
  const sent = lastSent
    ? review.comments.filter((c) => c.state === "sent" && c.round === lastSent.number)
    : [];
  return {
    total: files.length,
    viewedCount,
    // Comment rounds also take a number, so the pass count comes from delivered requests.
    round: lastSent ? delivered.length + 1 : null,
    requested: requests.length > 0,
    undelivered: latest?.verdict === "request_changes" && !latest.delivered ? latest : null,
    changed,
    untouchedViewed: files.filter((f) => viewed.has(f.path) && !touched.has(f.path)).length,
    sent: sent.length,
    addressed: sent.filter((c) => c.addressed).length,
    notes: review.notes.length,
    openNotes: review.notes.filter((n) => !n.resolved).length,
  };
}

export function ReviewChecklist({ view, fallback }: ReviewChecklistProps) {
  const wsId = view.item.id;
  const git = useWorkstreamGitActions(view);
  const review = useReview(wsId).data;
  const sessions = useWorkstreamSessions(wsId);
  const implement = newestImplementSession(sessions.data ?? []);
  const agentLabel = harnessLabel(implement?.harness);
  const delivery = useReviewDelivery(wsId, implement, agentLabel);
  const s = useMemo(() => (review ? summarize(review) : null), [review]);

  // Once changes were requested the sidebar tracks the fix round, working or not.
  if (!review || !s || s.total === 0 || (git.implementWorking && !s.requested)) return fallback;

  const target = { projectPath: view.item.projectPath, workstreamId: wsId };
  const status =
    review.status === "approved"
      ? "Approved"
      : s.round
        ? `Round ${s.round}`
        : s.viewedCount === 0
          ? "Not started"
          : "In progress";
  const pct = (s.viewedCount / s.total) * 100;

  return (
    <div className="flex h-full min-h-0 flex-col bg-panel">
      <div className="flex h-[38px] shrink-0 items-center gap-2 border-b border-edge px-[14px]">
        <Icon name="review" size="sm" className="text-ink-4" />
        <span className="min-w-0 truncate text-ui font-semibold text-ink">
          Review · {status} <span className="font-normal tabular-nums text-ink-dim">· {s.viewedCount} / {s.total} viewed</span>
        </span>
      </div>
      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-[18px] py-4">
        <span
          role="progressbar"
          aria-label="Files viewed"
          aria-valuemin={0}
          aria-valuemax={s.total}
          aria-valuenow={s.viewedCount}
          className="block h-1 overflow-hidden rounded-[3px] bg-panel-2"
        >
          <span className="block h-full rounded-[3px] bg-ink" style={{ width: `${pct}%` }} />
        </span>
        {s.undelivered && (
          <button
            type="button"
            disabled={delivery.delivering}
            onClick={() => {
              const round = s.undelivered;
              if (round?.message) void delivery.deliver(round, round.message);
            }}
            className="rounded-control disabled:opacity-60"
          >
            <Badge tone="bad">Not delivered · Resend</Badge>
          </button>
        )}
        {s.round !== null && (
          <p className="text-ui text-ink-2">
            Your comments{" "}
            <span className="tabular-nums text-ink">
              {s.addressed} / {s.sent}
            </span>{" "}
            addressed
          </p>
        )}
        {s.notes > 0 && (
          <p className="text-ui text-ink-2">
            {plural(s.notes, "note")} from {agentLabel} · {s.openNotes} unresolved
          </p>
        )}
        {s.round !== null && s.changed.length > 0 && (
          <div className="space-y-1">
            <div className="text-label text-ink-dim">Changed since your review</div>
            <ul>
              {s.changed.map((f) => (
                <li key={f.path}>
                  <button
                    type="button"
                    onClick={() => openReview({ ...target, file: f.path })}
                    title={f.path}
                    className={rowClass({ tier: "compact", className: "flex w-full gap-2 px-1.5 text-left" })}
                  >
                    <span className="min-w-0 flex-1 truncate font-mono text-meta text-ink">{basename(f.path)}</span>
                    <span className="shrink-0 text-meta tabular-nums">
                      <span className="text-diff-add">+{f.additions}</span>{" "}
                      <span className="text-diff-del">−{f.deletions}</span>
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        {s.round !== null && s.untouchedViewed > 0 && (
          <p className="text-meta text-ink-dim">
            {plural(s.untouchedViewed, "file")} untouched, still viewed
          </p>
        )}
      </div>
      <div className="shrink-0 space-y-2 border-t border-edge-2 px-[18px] py-2.5">
        {git.implementWorking && (
          <p className="text-meta text-ink-dim">
            {s.round ? `Ship unlocks when ${agentLabel} finishes round ${s.round}` : `Ship unlocks when ${agentLabel} finishes`}
          </p>
        )}
        <div className="flex items-center gap-2">
          <span className="flex-1" />
          <Button
            size="sm"
            variant="primary"
            onClick={() => openReview(s.round && s.changed.length > 0 ? { ...target, file: s.changed[0].path } : target)}
          >
            {s.round && s.changed.length > 0 ? `Review ${plural(s.changed.length, "updated file")}` : "Review changes"}
          </Button>
          {!git.hasLivePr && (
            <Button size="sm" disabled={!git.canShip} busy={git.shipping} onClick={git.doShip}>
              <Icon name="ship" size="sm" />
              Ship
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
