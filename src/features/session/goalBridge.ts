import { events, type GoalChanged } from "@/lib/bindings";
import { goalNotice } from "@/lib/domain/goals";
import type { EventBridge } from "@/lib/events";
import { log } from "@/lib/log";
import { findWorkstreamForNotify, notifyGoal } from "@/lib/notifications";
import { isAutoAllowed, isSessionVisible, reattachRunningGoal, setSessionGoal } from "./sessionStore";

// The last payload per live goal, so a notice reads one transition; an ended goal sends no more.
const previous = new Map<string, GoalChanged>();

const ENDED = new Set(["met", "exhausted", "failed", "stopped"]);

function onGoalChanged(payload: GoalChanged): void {
  const { goalId, sessionId, workstreamId, status, step, pauseReason, iteration, maxIterations } = payload;
  // Mirrors `Session.goal`, which never carries an ended goal.
  const brief = ENDED.has(status) ? null : { id: goalId, status, step, pauseReason, iteration, maxIterations };
  void reattachRunningGoal(sessionId, brief);
  setSessionGoal(sessionId, brief);
  const notice = goalNotice(previous.get(goalId), payload);
  if (ENDED.has(status)) previous.delete(goalId);
  else previous.set(goalId, payload);
  const autoAllowed = step === "waiting" && payload.reason != null && isAutoAllowed(sessionId, payload.reason);
  if (notice && !autoAllowed) void announce(sessionId, workstreamId, notice);
}

async function announce(
  sessionId: string,
  workstreamId: string,
  notice: { title: string; body: string },
): Promise<void> {
  try {
    if (await isSessionVisible(sessionId, workstreamId)) return;
    const ws = findWorkstreamForNotify(workstreamId);
    if (!ws) return;
    await notifyGoal({ workstreamId, projectPath: ws.projectPath, workstreamTitle: ws.title, ...notice });
  } catch (e) {
    log.warn("goal", "a goal notification failed", e);
  }
}

/** Goal notices fire on every device, unlike a session's own asks. */
export const goalBridge: EventBridge = () => [events.goalChanged.listen(({ payload }) => onGoalChanged(payload))];
