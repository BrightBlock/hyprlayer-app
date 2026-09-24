import { cx } from "@/components/ui";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";

import { PhaseSection } from "./PhaseSection";
import type { PlanTracker } from "./planTracker";

export function PlanChip({ className, children }: { className: string; children: string }) {
  return (
    <span className={cx("rounded-control px-[9px] py-[3px] text-[11.5px] font-semibold leading-none", className)}>
      {children}
    </span>
  );
}

// Amber for the states that wait on the person: a draft, or a plan the agent is asking them to approve.
const ATTENTION = new Set(["draft", "awaiting approval"]);

function statusChip(status: string | null) {
  const label = status || "draft";
  const text = label[0].toUpperCase() + label.slice(1);
  return ATTENTION.has(label.toLowerCase()) ? (
    <PlanChip className="bg-tag-draft-bg text-warn-ink">{text}</PlanChip>
  ) : (
    <PlanChip className="bg-panel-2 text-ink-2">{text}</PlanChip>
  );
}

const noJump = () => {};

export function PlanPreviewDraft({
  tracker,
  changed,
  reduceMotion,
}: {
  tracker: PlanTracker;
  changed: ReadonlySet<string>;
  reduceMotion: boolean;
}) {
  const flash = (key: string) => !reduceMotion && changed.has(key);
  return (
    <div className="px-[18px] py-5">
      <div className={cx("-mx-2 rounded-control px-2", flash("header") && "animate-plan-flash")}>
        <h2 className="mb-2.5 text-[20px] font-bold leading-[1.2] tracking-[-0.01em] text-ink">{tracker.title}</h2>
        <div className="mb-[18px] flex flex-wrap gap-1.5">
          <PlanChip className="bg-panel-2 text-ink">Plan</PlanChip>
          {statusChip(tracker.status)}
          {tracker.author && <PlanChip className="bg-panel-2 font-normal text-ink-dim">{tracker.author}</PlanChip>}
        </div>
      </div>
      {/* Plans without phases (Claude, codex, free-form thoughts plans) render their markdown instead. */}
      {tracker.phases.length === 0 ? (
        <div
          className={cx(
            "-mx-2 rounded-control px-2 text-[13px] leading-[1.6] text-ink-2",
            flash("body") && "animate-plan-flash",
          )}
        >
          <MarkdownPreview body={tracker.body} />
        </div>
      ) : (
        <>
          {tracker.overview && (
            <div className={cx("-mx-2 mb-[18px] rounded-control px-2", flash("overview") && "animate-plan-flash")}>
              <div className="lbl mb-1.5">Overview</div>
              <div className="text-[13px] leading-[1.6] text-ink-2">
                <MarkdownPreview body={tracker.overview} />
              </div>
            </div>
          )}
          <div className="space-y-[18px]">
            {tracker.phases.map((phase) => (
              <PhaseSection
                key={`${phase.label}:${phase.name}`}
                phase={phase}
                showState={false}
                pulse={false}
                flash={flash(`phase:${phase.label}`)}
                onJump={noJump}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
