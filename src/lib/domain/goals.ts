import type {
  Goal,
  GoalBrief,
  GoalChanged,
  GoalCheck,
  GoalCheckOutcome,
  GoalCheckResult,
  GoalDetail,
  GoalEndReason,
  GoalIteration,
  GoalLimits,
  GoalPatch,
  GoalPauseReason,
  GoalSource,
  GoalSpec,
  GoalStatus,
  GoalStep,
  GoalTarget,
  DeriveGoalRequest,
  StartGoalRequest,
} from "@/lib/bindings";
import { formatTokens, formatUsd } from "@/lib/format";
import type { WorkstreamActivity } from "./activity";

export type {
  DeriveGoalRequest,
  Goal,
  GoalBrief,
  GoalChanged,
  GoalCheck,
  GoalCheckOutcome,
  GoalCheckResult,
  GoalDetail,
  GoalEndReason,
  GoalIteration,
  GoalLimits,
  GoalPatch,
  GoalPauseReason,
  GoalSource,
  GoalSpec,
  GoalStatus,
  GoalStep,
  GoalTarget,
  StartGoalRequest,
};

export function isLiveGoal(goal: Pick<Goal, "status">): boolean {
  return goal.status === "running" || goal.status === "paused";
}

export const GOAL_STATUS_LABEL: Record<GoalStatus, string> = {
  running: "Running",
  paused: "Paused",
  met: "Met",
  exhausted: "Stopped at limit",
  failed: "Failed",
  stopped: "Stopped",
};

export const GOAL_STEP_LABEL: Record<GoalStep, string> = {
  working: "working",
  checking: "checking",
  waiting: "waiting on you",
  resuming: "resuming",
  pausing: "pausing",
  stopping: "stopping",
};

// Read after "Goal needs you: ", so each is a lowercase fragment.
export const GOAL_PAUSE_REASON_LABEL: Record<GoalPauseReason, string> = {
  user: "paused by you",
  needs_human: "a decision is needed",
  turn_failures: "turns kept failing",
  harness_crashes: "the agent kept crashing",
  restart_loop: "restarts made no progress",
  relaunch_failed: "the session could not relaunch",
};

export const GOAL_END_REASON_LABEL: Record<GoalEndReason, string> = {
  checks_passed: "checks passed",
  iteration_limit: "iteration limit",
  budget_limit: "budget limit",
  token_limit: "token limit",
  time_limit: "time limit",
  stalled: "no progress",
  checks_broken: "checks broken",
  stopped_by_user: "stopped by you",
  session_ended: "session ended",
  workstream_archived: "workstream archived",
};

// The server validates exactly one cap per harness: dollars, or tokens on Codex.
export function goalBudgetUnit(goal: Pick<Goal, "limits">): "usd" | "tokens" {
  return goal.limits.maxCostUsd != null ? "usd" : "tokens";
}

export function goalSpent(goal: Pick<Goal, "limits" | "spentCostUsd" | "spentTokens">): string {
  return goalBudgetUnit(goal) === "usd"
    ? formatUsd(goal.spentCostUsd ?? 0)
    : `${formatTokens(goal.spentTokens)} tokens`;
}

// The server's clock runs while running and not waiting on an ask; `activeMs` is as of `updatedAt`.
export function goalActiveMs(
  goal: Pick<Goal, "status" | "step" | "activeMs" | "updatedAt" | "limits">,
  nowMs: number,
): number {
  const ticking = goal.status === "running" && goal.step !== "waiting" && goal.updatedAt != null;
  const since = ticking ? Math.max(0, nowMs - (goal.updatedAt ?? nowMs)) : 0;
  return Math.min(goal.activeMs + since, goal.limits.maxActiveSecs * 1000);
}

export function formatActiveTime(ms: number): string {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

export function goalChip(goal: Goal, nowMs: number): string {
  const phase = goal.step ? GOAL_STEP_LABEL[goal.step] : GOAL_STATUS_LABEL[goal.status].toLowerCase();
  return [
    `${goal.iteration}/${goal.limits.maxIterations}`,
    phase,
    goalSpent(goal),
    formatActiveTime(goalActiveMs(goal, nowMs)),
  ].join(" · ");
}

export type GoalMeters = { iterations: number; spend: number; time: number };

const fraction = (used: number, max: number | null | undefined) =>
  max && max > 0 ? Math.min(1, Math.max(0, used / max)) : 0;

export function goalMeters(goal: Goal, nowMs: number = goal.updatedAt ?? 0): GoalMeters {
  const { limits } = goal;
  return {
    iterations: fraction(goal.iteration, limits.maxIterations),
    spend:
      goalBudgetUnit(goal) === "usd"
        ? fraction(goal.spentCostUsd ?? 0, limits.maxCostUsd)
        : fraction(goal.spentTokens, limits.maxTokens),
    time: fraction(goalActiveMs(goal, nowMs), limits.maxActiveSecs * 1000),
  };
}

export type GoalControl = "pause" | "resume" | "stop" | "edit" | "extend" | "startAgain";

export const GOAL_CONTROL_LABEL: Record<GoalControl, string> = {
  pause: "Pause",
  resume: "Resume",
  stop: "Stop",
  edit: "Edit…",
  extend: "Extend…",
  startAgain: "Start again…",
};

// The server refuses update and resume on failed and stopped goals (`goal_finished`).
const CONTROLS: Record<GoalStatus, GoalControl[]> = {
  running: ["pause", "stop", "edit"],
  paused: ["resume", "stop", "edit"],
  exhausted: ["extend", "startAgain"],
  failed: ["startAgain"],
  stopped: ["startAgain"],
  met: [],
};

export function goalControls(goal: Pick<Goal, "status">): GoalControl[] {
  return CONTROLS[goal.status];
}

const GOAL_DRAFT = /^\s*\/goal(?:\s+([\s\S]*))?$/;

export function goalDraft(text: string): string | null {
  const match = GOAL_DRAFT.exec(text);
  return match ? (match[1] ?? "").trim() : null;
}

export type GoalNotice = { title: string; body: string };

export function goalNotice(prev: GoalChanged | null | undefined, next: GoalChanged): GoalNotice | null {
  const entered = prev?.status !== next.status;
  const reason = next.reason ?? "";
  if (entered && next.status === "met") return { title: "Goal met", body: reason };
  if (entered && next.status === "exhausted") {
    return {
      title: "Goal stopped at its limit",
      body: next.endReason ? GOAL_END_REASON_LABEL[next.endReason] : "",
    };
  }
  if (entered && next.status === "failed") {
    const why = next.endReason ? GOAL_END_REASON_LABEL[next.endReason] : "unknown";
    return { title: `Goal failed: ${why}`, body: reason };
  }
  if (next.status === "paused" && next.pauseReason && next.pauseReason !== "user") {
    if (!entered && prev?.pauseReason === next.pauseReason) return null;
    return { title: `Goal needs you: ${GOAL_PAUSE_REASON_LABEL[next.pauseReason]}`, body: reason };
  }
  if (next.status === "running" && next.step === "waiting" && (entered || prev?.step !== "waiting")) {
    return { title: "Goal needs you", body: reason };
  }
  return null;
}

// Consulted before a session's own rules; null defers to them.
export function goalActivity(
  brief: Pick<GoalBrief, "status" | "step" | "pauseReason"> | null | undefined,
): WorkstreamActivity | null {
  if (!brief) return null;
  if (brief.status === "running") return brief.step === "waiting" ? "needs-you" : "working";
  if (brief.status === "paused") return brief.pauseReason && brief.pauseReason !== "user" ? "needs-you" : null;
  if (brief.status === "met") return "review";
  return null;
}

export function liveGoalForPlan(goals: readonly Goal[] | null | undefined, planPath: string): Goal | null {
  return goals?.find((g) => isLiveGoal(g) && g.sourceRef === planPath) ?? null;
}

export function workstreamGoalLabel(brief: Pick<GoalBrief, "iteration" | "maxIterations">): string {
  return `goal ${brief.iteration}/${brief.maxIterations}`;
}

export type CommandOutcome = { command: string; outcome: GoalCheckOutcome; exitCode: number | null };

// Results carry a label, not the command: the runner emits one per Command check, in check order.
// An explicit label that no longer matches means the checks were edited since that run.
function commandResults(checks: GoalCheck[], results: GoalCheckResult[]): CommandOutcome[] {
  const commands = checks.filter((c) => c.kind === "command");
  const ran = results.filter((r) => r.kind === "command");
  return ran.flatMap((result, i) => {
    const check = commands[i];
    if (!check || (check.label?.trim() && check.label.trim() !== result.label)) return [];
    return [{ command: check.command, outcome: result.outcome, exitCode: result.exitCode }];
  });
}

// The newest outcome of each command, over every finished iteration.
export function commandOutcomes(detail: Pick<GoalDetail, "goal" | "iterations">): CommandOutcome[] {
  const newest = new Map<string, CommandOutcome>();
  for (const iteration of detail.iterations) {
    for (const outcome of commandResults(detail.goal.checks, iteration.results)) newest.set(outcome.command, outcome);
  }
  return [...newest.values()];
}

// The longest command the title holds, so `pnpm test:e2e` never reads `pnpm test`'s outcome.
export function criterionOutcome(
  criterion: { title: string; group: string | null },
  outcomes: readonly CommandOutcome[],
): CommandOutcome | null {
  if (criterion.group === "manual") return null;
  let best: CommandOutcome | null = null;
  for (const o of outcomes) {
    if (criterion.title.includes(o.command) && o.command.length > (best?.command.length ?? 0)) best = o;
  }
  return best;
}
