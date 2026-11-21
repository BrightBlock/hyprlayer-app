import { create } from "zustand";

import type { Goal, GoalSource, GoalSpec, Harness, Stage } from "./bindings";

export type GoalSheetTarget =
  | { sessionId: string }
  | {
      newSession: { stage: Stage; shellId?: number; attachedDocs?: string[]; permissionMode?: string; harness?: Harness };
    };

// A `spec` skips `derive_goal`, so a caller that already derived one keeps its scope and unticks.
export type GoalSheetRequest = {
  mode: "start" | "edit" | "extend";
  workstreamId: string;
  source: GoalSource;
  text?: string;
  planPath?: string;
  phases?: string[];
  researchPath?: string;
  findings?: string[];
  manual?: string[];
  withValidation?: boolean;
  spec?: GoalSpec;
  target: GoalSheetTarget;
  goalId?: string;
};

type GoalSheetState = { request: GoalSheetRequest | null };

export const useGoalSheet = create<GoalSheetState>(() => ({ request: null }));

export function openGoalSheet(request: GoalSheetRequest): void {
  useGoalSheet.setState({ request });
}

export function closeGoalSheet(): void {
  useGoalSheet.setState({ request: null });
}

type GoalStartedHandler = (goal: Goal, request: GoalSheetRequest) => void;

const goalStartedHandlers: GoalStartedHandler[] = [];

export function onGoalStarted(handler: GoalStartedHandler): void {
  goalStartedHandlers.push(handler);
}

export function notifyGoalStarted(goal: Goal, request: GoalSheetRequest): void {
  for (const handler of goalStartedHandlers) handler(goal, request);
}
