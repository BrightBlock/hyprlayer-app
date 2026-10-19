import { Button, Spinner, cx, rowClass } from "@/components/ui";
import { Icon } from "@/components/Icon";
import type { Goal, GoalIteration } from "@/lib/bindings";
import { GOAL_CONTROL_LABEL, GOAL_STATUS_LABEL, GOAL_STEP_LABEL, goalControls } from "@/lib/domain/goals";
import { formatElapsed } from "@/lib/format";
import { useTickingNow } from "@/lib/useTickingNow";
import { docTitle } from "./toolDisplay";
import { goalActiveSince, goalMeterValues, useGoalControl, useSessionGoal } from "./sessionGoal";
import { useSessionStore } from "./sessionStore";
import { GoalCheckList, iterationSpend } from "./TranscriptView";
import { requestTranscriptJump } from "./transcriptJump";

function sourceLine(goal: Goal): string {
  const title = goal.sourceRef ? docTitle(goal.sourceRef) : null;
  switch (goal.source) {
    case "composer":
      return "Ad-hoc goal";
    case "plan":
    case "plan_phase":
      return [`From plan ${title ?? ""}`.trim(), goal.sourcePhase].filter(Boolean).join(" · ");
    case "research":
      return `From research ${title ?? ""}`.trim();
    case "validation":
      return "From validation findings";
  }
}

function startedLine(goal: Goal): string | null {
  if (goal.startedAt === null) return null;
  const at = new Date(goal.startedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return goal.author ? `started ${at} by ${goal.author.device}` : `started ${at}`;
}

function Tile({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-card border border-edge bg-panel-2/40 px-2.5 py-2">
      <div className="text-label text-ink-dim">{label}</div>
      <div className="mt-0.5 truncate text-body font-medium tabular-nums text-ink" title={value}>
        {value}
      </div>
    </div>
  );
}

function OutcomeIcon({ iteration }: { iteration: GoalIteration }) {
  if (iteration.outcome === null) return iteration.endedAt === null ? <Spinner size="xs" /> : <Icon name="paused" size="xs" />;
  if (iteration.outcome === "pass") return <Icon name="check" size="xs" className="text-ok" />;
  return <Icon name="failed" size="xs" className="text-bad-ink" />;
}

// The transcript divider is the iteration's prompt item, matched by the id both carry.
function jumpToIteration(sessionId: string, iteration: GoalIteration): void {
  const items = useSessionStore.getState().sessions[sessionId]?.items ?? [];
  const item = items.find((it) => it.kind === "user" && it.clientMessageId === iteration.clientMessageId);
  if (item) requestTranscriptJump(sessionId, { itemId: item.id });
}

function IterationRow({ goal, iteration, sessionId }: { goal: Goal; iteration: GoalIteration; sessionId: string }) {
  const done = iteration.startedAt !== null && iteration.endedAt !== null;
  const spend = iterationSpend(goal, iteration);
  return (
    <button
      type="button"
      onClick={() => jumpToIteration(sessionId, iteration)}
      data-testid="goal-timeline-row"
      className={rowClass({ tier: "compact", className: "flex w-full gap-2 px-2 text-left text-ui" })}
    >
      <span className="flex w-3 shrink-0 justify-center">
        <OutcomeIcon iteration={iteration} />
      </span>
      <span className="w-5 shrink-0 font-mono text-ink">{iteration.n}</span>
      {done ? (
        <>
          <span className="w-14 shrink-0 font-mono text-ink-dim">
            {formatElapsed(iteration.endedAt! - iteration.startedAt!)}
          </span>
          <span className="w-16 shrink-0 font-mono text-ink-dim">{spend ?? ""}</span>
        </>
      ) : (
        <span className="shrink-0 text-ink-dim">{goal.step ? GOAL_STEP_LABEL[goal.step] : "working"}…</span>
      )}
      <span className="min-w-0 flex-1 truncate text-ink-dim" title={iteration.reason ?? undefined}>
        {iteration.reason}
      </span>
    </button>
  );
}

/** The right aside's Goal view: the focused session's newest goal. */
export function GoalAside({ sessionId, workstreamId }: { sessionId: string | undefined; workstreamId: string }) {
  const { goal, detail } = useSessionGoal(sessionId, workstreamId);
  const now = useTickingNow(goal ? goalActiveSince(goal) : null);
  const control = useGoalControl();
  if (!goal || !sessionId) return null;

  const iterations = detail?.iterations ?? [];
  const lastChecked = [...iterations].reverse().find((it) => it.results.length > 0) ?? null;
  const values = goalMeterValues(goal, now);
  const pill = [GOAL_STATUS_LABEL[goal.status], goal.step && GOAL_STEP_LABEL[goal.step]].filter(Boolean).join(" · ");

  return (
    <div data-testid="goal-aside" className="flex h-full min-h-0 flex-col overflow-y-auto p-3 text-ui">
      <div className="flex flex-wrap items-center gap-2">
        <Icon name="goal" size="sm" className="text-ink-dim" />
        <span
          className={cx(
            "rounded-control px-1.5 py-0.5 font-medium",
            goal.status === "running" ? "bg-accent-soft text-ink" : "bg-panel-2 text-ink-2",
          )}
        >
          {pill}
        </span>
        <div className="ml-auto flex flex-wrap gap-1.5">
          {goalControls(goal).map((c) => (
            <Button
              key={c}
              size="sm"
              disabled={control.pending && (c === "pause" || c === "resume" || c === "stop")}
              onClick={() => control.run(c, goal)}
            >
              {GOAL_CONTROL_LABEL[c]}
            </Button>
          ))}
        </div>
      </div>
      <p className="mt-3 text-body text-ink">{goal.condition}</p>
      <p className="mt-0.5 text-ui-sm text-ink-dim">
        {[sourceLine(goal), startedLine(goal)].filter(Boolean).join(" · ")}
      </p>

      <div className="mt-3 grid grid-cols-3 gap-2">
        <Tile label="Iterations" value={`${goal.iteration} / ${goal.limits.maxIterations}`} />
        <Tile label="Spend" value={values.spend} />
        <Tile label="Active time" value={values.time} />
      </div>

      <div className="mt-4 text-label text-ink-dim">Iterations</div>
      <div className="mt-1">
        {detail === null ? (
          <Spinner size="sm" />
        ) : (
          iterations.map((it) => <IterationRow key={it.n} goal={goal} iteration={it} sessionId={sessionId} />)
        )}
      </div>

      {lastChecked && (
        <>
          <div className="mt-4 text-label text-ink-dim">Last check</div>
          <div className="mt-1.5">
            <GoalCheckList goal={goal} iteration={lastChecked} />
          </div>
        </>
      )}
    </div>
  );
}
