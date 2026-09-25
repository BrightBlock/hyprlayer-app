import { ExternalLink, FileText, MessageSquare } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { ICON } from "@/components/lucide";
import { Button } from "@/components/ui";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { useReducedMotion } from "@/lib/useReducedMotion";

import { PhaseSection } from "./PhaseSection";
import { PlanPreviewDraft } from "./PlanPreviewDraft";
import { PlanProgressHeader } from "./PlanProgressHeader";
import type { JumpTarget, PlanTracker } from "./planTracker";

const MANUAL_SCROLL_HOLD_MS = 10_000;

type Props = {
  tracker: PlanTracker;
  diff: { additions: number; deletions: number } | null;
  // Open in Library for a thoughts doc, Open file for one outside it, Show in chat for an inline plan.
  headerAction: { label: string; icon: "external" | "chat"; onClick: () => void } | null;
  onApprove: () => void;
  approving?: boolean;
  // Null hides the button; `blocked` disables it with that reason as the tooltip.
  approve?: { blocked: string | null } | null;
  changed?: ReadonlySet<string>;
  onClose?: () => void;
  onReviewChanges: () => void;
  onJump: (target: JumpTarget) => void;
};

const NO_CHANGES: ReadonlySet<string> = new Set();

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

// Announces completions only; an aria-live list would read out every status tick.
function useCompletionAnnouncement(tracker: PlanTracker): string {
  const seen = useRef<Set<string> | null>(null);
  const [message, setMessage] = useState("");
  const done = tracker.phases.flatMap((p) => p.tasks.filter((t) => t.state === "done"));
  const doneKey = done.map((t) => t.key).join("\n");
  useEffect(() => {
    const keys = new Set(done.map((t) => t.key));
    const previous = seen.current;
    seen.current = keys;
    if (!previous) return;
    const fresh = done.filter((t) => !previous.has(t.key));
    if (fresh.length > 0) setMessage(`Completed: ${fresh.map((t) => t.title).join(", ")}`);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the done set
  }, [doneKey]);
  return message;
}

export function PlanPanel({
  tracker,
  diff,
  headerAction,
  onApprove,
  approving,
  approve = { blocked: null },
  changed = NO_CHANGES,
  onClose,
  onReviewChanges,
  onJump,
}: Props) {
  const reduceMotion = useReducedMotion();
  const tracking = tracker.mode === "tracking";
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastManualScroll = useRef(0);
  const announcement = useCompletionAnnouncement(tracker);
  const currentKey = tracker.current ? `${tracker.current.taskKey ?? ""}:${tracker.current.title}` : "";

  useEffect(() => {
    if (!tracking || !currentKey) return;
    if (Date.now() - lastManualScroll.current < MANUAL_SCROLL_HOLD_MS) return;
    const row = scrollRef.current?.querySelector<HTMLElement>('[data-current="true"]');
    row?.scrollIntoView?.({ block: "nearest", behavior: reduceMotion ? "auto" : "smooth" });
  }, [tracking, currentKey, reduceMotion]);

  const markManualScroll = () => {
    lastManualScroll.current = Date.now();
  };

  return (
    <div
      className="flex h-full min-h-0 flex-col bg-panel"
      onKeyDown={(e) => {
        if (e.key !== "Escape" || !onClose) return;
        e.stopPropagation();
        onClose();
      }}
    >
      <div className="flex h-[38px] shrink-0 items-center gap-2 border-b border-edge px-[14px]">
        <FileText {...ICON} size={14} className="text-ink-4" />
        <span className="text-ui font-semibold text-ink">Plan preview</span>
        <span className="flex-1" />
        {headerAction && (
          <button
            type="button"
            onClick={headerAction.onClick}
            className="inline-flex h-[26px] items-center gap-1.5 rounded-control border border-edge px-2 text-ui-sm font-semibold text-ink-2 hover:bg-panel-2 hover:text-ink"
          >
            {headerAction.label}
            {headerAction.icon === "chat" ? <MessageSquare {...ICON} size={12} /> : <ExternalLink {...ICON} size={12} />}
          </button>
        )}
      </div>

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto"
        onWheel={markManualScroll}
        onTouchMove={markManualScroll}
        onPointerDown={markManualScroll}
        onKeyDown={markManualScroll}
      >
        {tracking ? (
          <>
            <PlanProgressHeader tracker={tracker} reduceMotion={reduceMotion} />
            <div className="space-y-[18px] px-[18px] pb-[18px] pt-5">
              {tracker.phases.map((phase) => (
                <PhaseSection
                  key={`${phase.label}:${phase.name}`}
                  phase={phase}
                  showState
                  pulse={tracker.pulsing && !reduceMotion}
                  flash={!reduceMotion && changed.has(`phase:${phase.label}`)}
                  onJump={onJump}
                />
              ))}
              {tracker.phases.length === 0 && (
                <div className="text-body leading-prose text-ink-2">
                  <MarkdownPreview body={tracker.body} />
                </div>
              )}
            </div>
          </>
        ) : (
          <PlanPreviewDraft tracker={tracker} changed={changed} reduceMotion={reduceMotion} />
        )}
        <div className="sr-only" aria-live="polite">
          {announcement}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 border-t border-edge-2 px-[18px] py-2.5">
        {tracking ? (
          <>
            {tracker.total > 0 && (
              <span className="text-meta tabular-nums text-ink-4">
                {tracker.done} of {plural(tracker.total, "task")} done
              </span>
            )}
            <span className="flex-1" />
            {tracker.allDone ? (
              <Button variant="primary" size="sm" onClick={onReviewChanges}>
                Review changes
              </Button>
            ) : (
              diff && (
                <span className="flex items-center gap-1.5 text-meta tabular-nums">
                  <span className="text-diff-add">+{diff.additions}</span>
                  <span className="text-diff-del">−{diff.deletions}</span>
                </span>
              )
            )}
          </>
        ) : (
          <>
            {tracker.phases.length > 0 && (
              <span className="text-meta tabular-nums text-ink-4">
                {plural(tracker.phases.length, "phase")} · {tracker.done} of {plural(tracker.total, "task")} done
              </span>
            )}
            <span className="flex-1" />
            {approve && (
              <span title={approve.blocked ?? undefined}>
                <Button
                  variant="primary"
                  size="sm"
                  className="h-[26px] text-ui-sm font-semibold"
                  busy={approving}
                  disabled={approve.blocked !== null}
                  onClick={onApprove}
                >
                  Approve &amp; build
                </Button>
              </span>
            )}
          </>
        )}
      </div>
    </div>
  );
}
