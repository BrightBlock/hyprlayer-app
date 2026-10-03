import type { ComponentType } from "react";

export type ReadyCardProps = { sessionId: string; workstreamId: string };

// The review feature renders the card; main.tsx registers it so session never imports review components.
let ReadyCardComponent: ComponentType<ReadyCardProps> | null = null;

export function registerReadyCard(component: ComponentType<ReadyCardProps>): void {
  ReadyCardComponent = component;
}

export function readyCardComponent(): ComponentType<ReadyCardProps> | null {
  return ReadyCardComponent;
}
