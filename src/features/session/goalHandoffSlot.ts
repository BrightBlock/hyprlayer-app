import type { ComponentType } from "react";

import type { Goal } from "@/lib/bindings";

export type GoalHandoffProps = { sessionId: string; workstreamId: string; goal: Goal };

// The review feature renders the met card's hand-off; main.tsx registers it so session never imports review components.
let GoalHandoffComponent: ComponentType<GoalHandoffProps> | null = null;

export function registerGoalHandoff(component: ComponentType<GoalHandoffProps>): void {
  GoalHandoffComponent = component;
}

export function goalHandoffComponent(): ComponentType<GoalHandoffProps> | null {
  return GoalHandoffComponent;
}
