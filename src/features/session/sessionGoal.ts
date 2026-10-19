import type { Goal, GoalDetail, GoalSpec } from "@/lib/bindings";
import { formatActiveTime, goalActiveMs, goalBudgetUnit, type GoalControl } from "@/lib/domain/goals";
import { formatTokens, formatUsd } from "@/lib/format";
import { openGoalSheet } from "@/lib/goalSheet";
import { errorMessage } from "@/lib/ipc";
import { useGoal, useGoals, usePauseGoal, useResumeGoal, useStopGoal } from "@/lib/queries";
import { useToast } from "@/lib/toast";

/** The session's newest goal, live or ended, from the workstream's goal list. */
export function useSessionGoalRow(sessionId: string | null | undefined, workstreamId: string | null): Goal | null {
  const goals = useGoals(workstreamId, false, workstreamId !== null).data;
  return (sessionId && goals?.find((g) => g.sessionId === sessionId)) || null;
}

// Every surface of a session (chip, dividers, check cards, aside) reads this one detail.
export function useSessionGoal(
  sessionId: string | null | undefined,
  workstreamId: string | null,
): { goal: Goal | null; detail: GoalDetail | null } {
  const goal = useSessionGoalRow(sessionId, workstreamId);
  const detail = useGoal(goal?.id ?? null).data ?? null;
  return { goal, detail: detail && detail.goal.id === goal?.id ? detail : null };
}

/** Spend and active time against their caps, in the goal's own budget unit. */
export function goalMeterValues(goal: Goal, now: number): { spend: string; time: string } {
  const spend =
    goalBudgetUnit(goal) === "usd"
      ? `${formatUsd(goal.spentCostUsd ?? 0)} / ${formatUsd(goal.limits.maxCostUsd ?? 0)}`
      : `${formatTokens(goal.spentTokens)} / ${formatTokens(goal.limits.maxTokens ?? 0)}`;
  const time = `${formatActiveTime(goalActiveMs(goal, now))} / ${formatActiveTime(goal.limits.maxActiveSecs * 1000)}`;
  return { spend, time };
}

/** The clock `goalActiveMs` advances from, or null while it is stopped. */
export function goalActiveSince(goal: Goal): number | null {
  return goal.status === "running" && goal.step !== "waiting" ? goal.updatedAt : null;
}

function goalSpec(goal: Goal): GoalSpec {
  const { condition, workPrompt, source, sourceRef, sourcePhase, checks, limits } = goal;
  return { condition, workPrompt, source, sourceRef, sourcePhase, checks, limits };
}

const failed = (e: unknown) => useToast.getState().show("Goal", errorMessage(e), { tone: "bad" });

/** Runs one of `goalControls(goal)`; the sheet opens for the ones that need a spec. */
export function useGoalControl(): { run: (control: GoalControl, goal: Goal) => void; pending: boolean } {
  const pause = usePauseGoal();
  const resume = useResumeGoal();
  const stop = useStopGoal();
  const run = (control: GoalControl, goal: Goal) => {
    const base = { workstreamId: goal.workstreamId, source: goal.source, target: { sessionId: goal.sessionId } };
    switch (control) {
      case "pause":
        pause.mutate(goal.id, { onError: failed });
        return;
      case "resume":
        resume.mutate(goal.id, { onError: failed });
        return;
      case "stop":
        stop.mutate(goal.id, { onError: failed });
        return;
      case "edit":
      case "extend":
        openGoalSheet({ ...base, mode: control, goalId: goal.id });
        return;
      case "startAgain":
        openGoalSheet({ ...base, mode: "start", spec: goalSpec(goal) });
        return;
    }
  };
  return { run, pending: pause.isPending || resume.isPending || stop.isPending };
}
