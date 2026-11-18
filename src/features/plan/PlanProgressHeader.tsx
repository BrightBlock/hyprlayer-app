import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import type { RunningSubagent } from "@/features/session/subagent";

import type { StageBadge } from "./lifecycleSlot";
import { TaskBox } from "./PhaseSection";
import { setPhaseCollapsed, usePhaseCollapsed } from "./planPreviewStore";
import { phaseLine, progressLabel, type CurrentItem, type OpenManualCheck, type PlanTracker, type TrackedTask } from "./planTracker";

function subagentLine(s: RunningSubagent): string {
  const parts = [s.agent];
  if (s.steps) {
    parts.push(`${s.steps.done} of ${s.steps.total} steps`);
    if (s.steps.current) parts.push(s.steps.current);
  } else if (s.tools > 0) {
    parts.push(`${s.tools} tool${s.tools === 1 ? "" : "s"}`);
    if (s.lastTool) parts.push(s.lastTool);
  }
  return parts.join(" · ");
}

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
      {current.subagent && (
        <div className="mt-1 truncate text-meta tabular-nums text-ink-4">{subagentLine(current.subagent)}</div>
      )}
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

// Capped so a plan mid-build, with most checks open, doesn't push the lifecycle and phases off screen.
function OpenManualChecks({
  open,
  collapseKey,
  onCheckOff,
}: {
  open: OpenManualCheck[];
  collapseKey: string;
  onCheckOff?: (task: TrackedTask) => void;
}) {
  const collapsed = usePhaseCollapsed(collapseKey, false);
  return (
    <section aria-label="Open manual checks" className="mt-3 rounded-card border border-edge px-3 py-2.5">
      <button
        type="button"
        aria-expanded={!collapsed}
        onClick={() => setPhaseCollapsed(collapseKey, !collapsed)}
        className="lbl flex w-full items-center gap-1.5 text-left"
      >
        <Icon
          name="chevronDown"
          size="xs"
          className={cx("shrink-0 transition-transform", collapsed && "-rotate-90")}
        />
        Open manual checks <span className="tabular-nums">· {open.length}</span>
      </button>
      {!collapsed && (
        <ul className="mt-2 max-h-[220px] space-y-2 overflow-y-auto">
          {open.map(({ phaseLabel, phaseName, task }) => (
            <li key={task.key} className="flex items-start gap-[9px] text-body">
              {onCheckOff ? (
                <button
                  type="button"
                  aria-label={`Mark done: ${task.title}`}
                  onClick={() => onCheckOff(task)}
                  className="flex min-w-0 flex-1 items-start gap-[9px] text-left"
                >
                  <TaskBox state="pending" pulse={false} />
                  <span className="min-w-0 flex-1 break-words text-ink-2">{task.title}</span>
                </button>
              ) : (
                <>
                  <TaskBox state="pending" pulse={false} />
                  <span className="min-w-0 flex-1 break-words text-ink-2">{task.title}</span>
                </>
              )}
              <span
                title={phaseName}
                className="shrink-0 rounded-control border border-edge px-1 text-meta tabular-nums text-ink-4"
              >
                Phase {phaseLabel}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function PlanProgressHeader({
  tracker,
  reduceMotion,
  badge,
  onCheckOff,
}: {
  tracker: PlanTracker;
  reduceMotion: boolean;
  badge?: StageBadge;
  // Absent for plans the app can't write to (Claude's and inline plans).
  onCheckOff?: (task: TrackedTask) => void;
}) {
  const pct = Math.round(tracker.progress * 100);
  const shown = badge ?? {
    label: tracker.allDone ? "Done" : "Build",
    className: tracker.allDone ? "bg-ok" : "bg-info",
    detail: phaseLine(tracker),
  };
  return (
    <div className="px-[18px] pt-4">
      <h2 className="text-h3 text-ink">{tracker.title}</h2>
      <div className="mt-2 flex items-center gap-2">
        <span
          className={cx(
            "rounded-control px-[9px] py-[3px] text-ui-sm font-semibold leading-none text-accent-ink",
            shown.className,
          )}
        >
          {shown.label}
        </span>
        {shown.detail && <span className="text-meta tabular-nums text-ink-4">{shown.detail}</span>}
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
      {tracker.total > 0 && (
        <div className="mt-2 text-meta tabular-nums text-ink-4">{progressLabel(tracker)}</div>
      )}
      {tracker.current && (
        <CurrentTaskCard current={tracker.current} pulse={tracker.pulsing && !reduceMotion} />
      )}
      {tracker.openManual.length > 0 && (
        <OpenManualChecks
          open={tracker.openManual}
          collapseKey={`${tracker.title}\0open-manual`}
          onCheckOff={onCheckOff}
        />
      )}
    </div>
  );
}
