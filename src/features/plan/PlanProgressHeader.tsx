import { cx } from "@/components/ui";

import type { CurrentItem, PlanTracker } from "./planTracker";

export function CurrentTaskCard({
  current,
  pulse,
}: {
  current: CurrentItem;
  pulse: boolean;
}) {
  return (
    <div className="mt-3 rounded-card border border-edge bg-panel-2 px-3 py-2.5">
      <div className="lbl">Current task</div>
      <div className="mt-1 flex items-center gap-2">
        <span
          aria-hidden="true"
          className={cx(
            "size-[7px] shrink-0 rounded-full",
            current.state === "waiting" ? "bg-warn" : current.state === "failed" ? "bg-bad" : "bg-ink",
            pulse && "animate-plan-pulse",
          )}
        />
        <span className="min-w-0 flex-1 truncate text-body font-semibold text-ink">{current.title}</span>
      </div>
      {current.reason && (
        <div
          className={cx(
            "mt-1 text-meta",
            current.state === "waiting" ? "text-warn-ink" : "text-bad-ink",
          )}
        >
          {current.reason}
        </div>
      )}
    </div>
  );
}

export function PlanProgressHeader({
  tracker,
  reduceMotion,
}: {
  tracker: PlanTracker;
  reduceMotion: boolean;
}) {
  const pct = Math.round(tracker.progress * 100);
  return (
    <div className="px-[18px] pt-4">
      <h2 className="text-h3 text-ink">{tracker.title}</h2>
      <div className="mt-2 flex items-center gap-2">
        <span
          className={cx(
            "rounded-control px-[9px] py-[3px] text-ui-sm font-semibold leading-none text-white",
            tracker.allDone ? "bg-ok" : "bg-info",
          )}
        >
          {tracker.allDone ? "Done" : "Build"}
        </span>
        {tracker.currentPhase !== null && (
          <span className="text-meta tabular-nums text-ink-4">
            Phase {tracker.currentPhase} of {tracker.phases.length}
          </span>
        )}
      </div>
      <div
        role="progressbar"
        aria-label="Plan progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        className="mt-3 h-[5px] overflow-hidden rounded-[3px] bg-panel-2"
      >
        <div
          className="h-full rounded-[3px] bg-ink"
          style={{
            width: `${tracker.progress * 100}%`,
            transition: reduceMotion ? undefined : "width 200ms ease",
          }}
        />
      </div>
      {tracker.current && (
        <CurrentTaskCard current={tracker.current} pulse={tracker.pulsing && !reduceMotion} />
      )}
    </div>
  );
}
