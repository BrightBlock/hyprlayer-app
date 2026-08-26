import { ActivityDot } from "@/components/ActivityDot";
import type { WorkstreamActivity } from "@/lib/domain/activity";
import type { AgentState } from "@/features/multiplexer/engineState";
import { useWorkstreamAgentRollup } from "@/features/multiplexer/multiplexerStore";
import { useWorkstreamActivity } from "./sessionStore";

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
