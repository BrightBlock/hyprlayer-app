import { FileText } from "lucide-react";

import { ICON } from "@/components/lucide";
import { cx } from "@/components/ui";
import type { PlanCardProps } from "@/features/session/planCardSlot";
import { isInlinePlanKey } from "@/features/session/planPresentation";
import { docTitle } from "@/features/session/toolDisplay";
import { formatRelativeTime } from "@/lib/format";
import { useTickingNow } from "@/lib/useTickingNow";

import { closePlanPreview, openPlanPreview, usePlanPreviewOpen, useResolvedPlanSelection } from "./planPreviewStore";
import { usePlan } from "./usePlan";

export function PlanCard({ sessionId, workstreamId, toolUseId, planKey, text, superseded, latest }: PlanCardProps) {
  const { plan, mtimeMs } = usePlan(planKey, text);
  const now = useTickingNow(mtimeMs);
  const selection = useResolvedPlanSelection(sessionId, workstreamId);
  const open = usePlanPreviewOpen();
  const active =
    open &&
    (selection
      ? selection.key === planKey && (selection.followLatest ? !superseded : selection.cardId === toolUseId)
      : latest);

  const title = plan?.title ?? (isInlinePlanKey(planKey) ? "Plan" : docTitle(planKey));
  const phases = plan?.phases.length ?? 0;
  const meta = superseded
    ? "Plan · superseded"
    : [
        "Plan",
        phases > 0 && `${phases} phase${phases === 1 ? "" : "s"}`,
        mtimeMs !== null && formatRelativeTime(mtimeMs, now),
      ]
        .filter(Boolean)
        .join(" · ");

  return (
    <button
      type="button"
      data-plan-card={toolUseId}
      data-plan-card-active={active || undefined}
      data-tooluse-id={toolUseId}
      aria-label={`Preview plan: ${title}`}
      aria-expanded={active}
      onClick={() =>
        active
          ? closePlanPreview()
          : openPlanPreview(sessionId, { key: planKey, cardId: toolUseId, followLatest: !superseded })
      }
      className={cx(
        "@container flex w-full max-w-[420px] items-center gap-2.5 rounded-card border bg-panel px-[13px] py-[11px] text-left",
        "focus:outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent/60",
        active ? "border-ink shadow-ring-active" : "border-edge hover:bg-panel-2",
      )}
    >
      <FileText {...ICON} className="shrink-0 text-ink-dim" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-body font-semibold text-ink">{title}</span>
        <span className="block truncate text-meta tabular-nums text-ink-4">{meta}</span>
      </span>
      {!active && (
        <span className="hidden shrink-0 text-meta text-ink-4 @[340px]:inline">click to preview →</span>
      )}
    </button>
  );
}
