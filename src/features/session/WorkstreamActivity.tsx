import { ActivityDot } from "@/components/ActivityDot";
import type { WorkstreamActivity } from "@/lib/domain/activity";
import { workstreamGoalLabel } from "@/lib/domain/goals";
import type { AgentState } from "@/features/multiplexer/engineState";
import { useWorkstreamAgentRollup } from "@/features/multiplexer/multiplexerStore";
import type { ReactNode } from "react";
import { useWorkstreamActivity, useWorkstreamBuildPhase, useWorkstreamGoal } from "./sessionStore";

const ACTIVITY_PRIORITY: Record<WorkstreamActivity, number> = {
  error: 5,
  "needs-you": 4,
  working: 3,
  background: 2,
  review: 1,
  idle: 0,
};

const PTY_TO_ACTIVITY: Record<AgentState, WorkstreamActivity> = {
  blocked: "needs-you",
  working: "working",
  done: "review",
  idle: "idle",
};
const ptyToActivity = (state: AgentState | undefined): WorkstreamActivity =>
  state ? PTY_TO_ACTIVITY[state] : "idle";

export function WorkstreamActivityDot({
  workstreamId,
  idle = "hide",
}: {
  workstreamId: string;
  idle?: "hide" | "dot";
}) {
  const sessionActivity = useWorkstreamActivity(workstreamId);
  const ptyActivity = ptyToActivity(useWorkstreamAgentRollup(workstreamId));
  const activity =
    ACTIVITY_PRIORITY[ptyActivity] > ACTIVITY_PRIORITY[sessionActivity]
      ? ptyActivity
      : sessionActivity;
  return <ActivityDot activity={activity} idle={idle} />;
}

export function WorkstreamBuildPhase({
  workstreamId,
  fallback,
}: {
  workstreamId: string;
  fallback: ReactNode;
}) {
  const goal = useWorkstreamGoal(workstreamId);
  const phase = useWorkstreamBuildPhase(workstreamId);
  if (goal) {
    return (
      <span
        data-testid="sidebar-goal"
        className="shrink-0 text-meta tabular-nums text-ink-4"
        title={`goal · iteration ${goal.iteration} of ${goal.maxIterations}`}
      >
        {workstreamGoalLabel(goal)}
      </span>
    );
  }
  if (!phase) return fallback;
  return (
    <span className="shrink-0 text-meta tabular-nums text-ink-4" title={`building · phase ${phase}`}>
      <span className="sr-only">building · </span>phase {phase}
    </span>
  );
}
