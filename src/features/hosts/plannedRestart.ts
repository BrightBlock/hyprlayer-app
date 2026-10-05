import { onServerReconnected } from "@/lib/events";

// Set before this app stops a server; its disconnect event can land after the call returns.
let planned = false;

export function planRestart(): void {
  planned = true;
}

export function cancelPlannedRestart(): void {
  planned = false;
}

export function isPlannedRestart(): boolean {
  return planned;
}

onServerReconnected(cancelPlannedRestart);
