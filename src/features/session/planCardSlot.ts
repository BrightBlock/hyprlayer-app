import { createContext, type ComponentType } from "react";

import type { PlanRevision } from "./planRevisions";

export type PlanCardProps = { sessionId: string; workstreamId: string; toolUseId: string } & PlanRevision;

// The plan feature renders the card; main.tsx registers it so session never imports plan components.
let PlanCardComponent: ComponentType<PlanCardProps> | null = null;

export function registerPlanCard(component: ComponentType<PlanCardProps>): void {
  PlanCardComponent = component;
}

export function planCardComponent(): ComponentType<PlanCardProps> | null {
  return PlanCardComponent;
}

type PlanCards = { sessionId: string; workstreamId: string; revisions: ReadonlyMap<string, PlanRevision> };

export const PlanCardsContext = createContext<PlanCards | null>(null);
