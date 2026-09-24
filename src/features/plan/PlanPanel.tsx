import { ExternalLink, FileText } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { ICON } from "@/components/lucide";
import { Button, cx } from "@/components/ui";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { useReducedMotion } from "@/lib/useReducedMotion";

import { PhaseSection } from "./PhaseSection";
import { PlanProgressHeader } from "./PlanProgressHeader";
import type { JumpTarget, PlanTracker } from "./planTracker";

const MANUAL_SCROLL_HOLD_MS = 10_000;

type Props = {
  tracker: PlanTracker;
  diff: { additions: number; deletions: number } | null;
  onOpenInLibrary: () => void;
  onApprove: () => void;
  approving?: boolean;
  onReviewChanges: () => void;
  onJump: (target: JumpTarget) => void;
};

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function Chip({ className, children }: { className: string; children: string }) {
  return (
    <span className={cx("rounded-control px-[7px] py-[3px] text-[11.5px] font-semibold leading-none", className)}>
      {children}
    </span>
  );
}

function statusChip(status: string | null) {
  if (!status || status.toLowerCase() === "draft") {
    return <Chip className="bg-tag-draft-bg text-warn-ink">Draft</Chip>;
  }
  return <Chip className="bg-panel-2 text-ink-2">{status[0].toUpperCase() + status.slice(1)}</Chip>;
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
  onOpenInLibrary,
  onApprove,
  approving,
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
    <div className="flex h-full min-h-0 flex-col bg-panel">
      <div className="flex h-[38px] shrink-0 items-center gap-2 border-b border-edge px-[14px]">
        <FileText {...ICON} size={14} className="text-ink-4" />
        <span className="text-[12.5px] font-bold text-ink">Plan preview</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onOpenInLibrary}
          className="inline-flex h-[26px] items-center gap-1.5 rounded-control border border-edge px-2 text-[11.5px] font-semibold text-ink-2 hover:bg-panel-2 hover:text-ink"
        >
          Open in Library
          <ExternalLink {...ICON} size={12} />
        </button>
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
          <PlanProgressHeader tracker={tracker} reduceMotion={reduceMotion} />
        ) : (
          <div className="px-[18px] pt-4">
            <h2 className="text-[19px] font-bold leading-tight tracking-[-0.01em] text-ink">{tracker.title}</h2>
            <div className="mt-2 flex items-center gap-1.5">
              <Chip className="bg-panel-2 text-ink-2">Plan</Chip>
              {statusChip(tracker.status)}
              {tracker.author && <span className="ml-1 text-[11.5px] text-ink-4">{tracker.author}</span>}
            </div>
            {tracker.overview && (
              <div className="mt-4">
                <div className="lbl mb-1.5">Overview</div>
                <div className="text-[13px] text-ink-2">
                  <MarkdownPreview body={tracker.overview} />
                </div>
              </div>
            )}
          </div>
        )}

        <div className="space-y-[18px] px-[18px] pb-[18px] pt-5">
          {tracker.phases.map((phase) => (
            <PhaseSection
              key={`${phase.label}:${phase.name}`}
              phase={phase}
              showState={tracking}
              pulse={tracker.pulsing && !reduceMotion}
              onJump={onJump}
            />
          ))}
          {tracker.phases.length === 0 && (
            <p className="text-[13px] text-ink-4">No phases found. Add “## Phase 1: …” headings to the plan.</p>
          )}
        </div>
        <div className="sr-only" aria-live="polite">
          {announcement}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 border-t border-edge-2 px-[18px] py-2.5">
        {tracking ? (
          <>
            <span className="font-mono text-[10.5px] tabular-nums text-ink-4">
              {tracker.done} of {plural(tracker.total, "task")} done
            </span>
            <span className="flex-1" />
            {tracker.allDone ? (
              <Button variant="primary" size="sm" onClick={onReviewChanges}>
                Review changes
              </Button>
            ) : (
              diff && (
                <span className="flex items-center gap-1.5 font-mono text-[10.5px] tabular-nums">
                  <span className="text-diff-add">+{diff.additions}</span>
                  <span className="text-diff-del">−{diff.deletions}</span>
                </span>
              )
            )}
          </>
        ) : (
          <>
            <span className="font-mono text-[10.5px] tabular-nums text-ink-4">
              {plural(tracker.phases.length, "phase")} · {tracker.done} of {plural(tracker.total, "task")} done
            </span>
            <span className="flex-1" />
            <Button variant="primary" size="sm" busy={approving} onClick={onApprove}>
              Approve &amp; build
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
