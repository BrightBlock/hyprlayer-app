import type { ComponentType, ReactNode } from "react";

import type { WorkstreamView } from "@/lib/bindings";

// The stage a tracked plan is in, shown as the header's badge.
export type StageBadge = { label: string; className: string; detail: string | null };

// The plan panel lends its header and phase list; the review feature decides the stage and what sits between them.
export type LifecycleStepsProps = {
  view: WorkstreamView;
  sessionId: string;
  header: (badge: StageBadge) => ReactNode;
  phases: ReactNode;
};

// The review feature renders the stages; main.tsx registers them so plan never imports review components.
let LifecycleStepsComponent: ComponentType<LifecycleStepsProps> | null = null;

export function registerLifecycleSteps(component: ComponentType<LifecycleStepsProps>): void {
  LifecycleStepsComponent = component;
}

export function lifecycleStepsComponent(): ComponentType<LifecycleStepsProps> | null {
  return LifecycleStepsComponent;
}
