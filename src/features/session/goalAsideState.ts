import { useEffect } from "react";

import { useUiStore, type AsideView } from "@/lib/uiStore";
import type { GoalStatus } from "@/lib/domain/goals";
import { useSessionGoalRow } from "./sessionGoal";

/** The chosen view when it is available, else Plan, else Goal, else Changes. */
export function resolveAsideView(chosen: AsideView, available: { plan: boolean; goal: boolean }): AsideView {
  if (chosen === "changes" || available[chosen]) return chosen;
  return available.plan ? "plan" : available.goal ? "goal" : "changes";
}

/** True once per goal id, for a running goal with no plan beside it and no view picked this run. */
export function claimGoalAutoOpen(
  opened: Set<string>,
  goal: { id: string; status: GoalStatus } | null,
  context: { plan: boolean; chosen: boolean },
): boolean {
  if (!goal || goal.status !== "running" || context.plan || context.chosen || opened.has(goal.id)) return false;
  opened.add(goal.id);
  return true;
}

const autoOpened = new Set<string>();

export function useGoalAside(
  sessionId: string | undefined,
  workstreamId: string | null,
  planAvailable: boolean,
  narrow: boolean,
) {
  const goal = useSessionGoalRow(sessionId, workstreamId);
  const asideView = useUiStore((s) => s.asideView);
  const setView = useUiStore((s) => s.setAsideView);
  const goalId = goal?.id ?? null;
  const status = goal?.status ?? null;

  useEffect(() => {
    const running = goalId && status ? { id: goalId, status } : null;
    const chosen = useUiStore.getState().asideViewChosen;
    if (!claimGoalAutoOpen(autoOpened, running, { plan: planAvailable, chosen })) return;
    useUiStore.setState(
      narrow ? { asideView: "goal", narrowDrawer: "diff" } : { asideView: "goal", diffHidden: false },
    );
  }, [goalId, status, planAvailable, narrow]);

  const available = goal !== null;
  return { available, view: resolveAsideView(asideView, { plan: planAvailable, goal: available }), setView };
}
