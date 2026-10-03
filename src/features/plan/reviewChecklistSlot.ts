import type { ComponentType, ReactNode } from "react";

import type { WorkstreamView } from "@/lib/bindings";

// `fallback` is the plan panel, shown until the review is ready.
export type ReviewChecklistProps = { view: WorkstreamView; fallback: ReactNode };

// The review feature renders the checklist; main.tsx registers it so plan never imports review components.
let ReviewChecklistComponent: ComponentType<ReviewChecklistProps> | null = null;

export function registerReviewChecklist(component: ComponentType<ReviewChecklistProps>): void {
  ReviewChecklistComponent = component;
}

export function reviewChecklistComponent(): ComponentType<ReviewChecklistProps> | null {
  return ReviewChecklistComponent;
}
