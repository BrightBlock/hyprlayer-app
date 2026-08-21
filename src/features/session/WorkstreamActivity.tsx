/**
 * Leading/trailing activity dot for a workstream row, merging native-session
 * activity with any pty agents' rollup so e.g. a blocked CLI dominates a quiet
 * chat in the same workstream. The dot's colors and markup live in the shared
 * `ActivityDot` (components/ActivityDot.tsx); this is the store-subscribed
 * wrapper that resolves which activity to show it.
 */
import { ActivityDot } from "@/components/ActivityDot";
import type { WorkstreamActivity } from "@/lib/domain/activity";
import type { AgentState } from "@/features/multiplexer/engineState";
import { useWorkstreamAgentRollup } from "@/features/multiplexer/multiplexerStore";
import { useWorkstreamActivity } from "./sessionStore";

// Worst-first priority used to merge native-session and pty-agent activity into
// one dot, so e.g. a blocked CLI dominates a quiet chat in the same workstream.
const ACTIVITY_PRIORITY: Record<WorkstreamActivity, number> = {
  error: 5,
  "needs-you": 4,
  working: 3,
  background: 2,
  review: 1,
  idle: 0,
};

/** Map a pty agent's state onto the shared workstream-activity vocabulary. */
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
  // Merge native-session activity with any pty agents' rollup (the latter is
  // always "idle" when multiplexer is off, so this is a no-op then).
  const sessionActivity = useWorkstreamActivity(workstreamId);
  const ptyActivity = ptyToActivity(useWorkstreamAgentRollup(workstreamId));
  const activity =
    ACTIVITY_PRIORITY[ptyActivity] > ACTIVITY_PRIORITY[sessionActivity]
      ? ptyActivity
      : sessionActivity;
  return <ActivityDot activity={activity} idle={idle} />;
}
