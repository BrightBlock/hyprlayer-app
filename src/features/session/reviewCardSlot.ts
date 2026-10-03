import type { ComponentType, ReactNode } from "react";

export const REVIEW_PROMPT = "hyprlayer:review";

// `fallback` is the plain prompt card, for a message the review feature can't read.
export type ReviewCardProps = { text: string; workstreamId: string | null; fallback: ReactNode };

// The review feature renders the card; main.tsx registers it so session never imports review components.
let ReviewCardComponent: ComponentType<ReviewCardProps> | null = null;

export function registerReviewCard(component: ComponentType<ReviewCardProps>): void {
  ReviewCardComponent = component;
}

export function reviewCardComponent(): ComponentType<ReviewCardProps> | null {
  return ReviewCardComponent;
}
