
export type AppLocation = {
  pathname: string;
  search: Record<string, unknown>;
};

type WorkstreamTarget = { projectPath: string; workstreamId: string };

export type ReviewTab = "files" | "plan";

export type ReviewTarget = WorkstreamTarget & { tab?: ReviewTab; file?: string; line?: number };

let locationGetter: (() => AppLocation) | null = null;
let workstreamNavigator: ((target: WorkstreamTarget) => void) | null = null;
let reviewOpener: ((target: ReviewTarget) => void) | null = null;
let reviewCloser: (() => void) | null = null;
let promptOpener: ((prompt: string) => void) | null = null;

export function registerAppNav(impl: {
  currentLocation: () => AppLocation;
  goToWorkstream: (target: WorkstreamTarget) => void;
  openReview: (target: ReviewTarget) => void;
  closeReview: () => void;
  openPrompt: (prompt: string) => void;
}): void {
  locationGetter = impl.currentLocation;
  workstreamNavigator = impl.goToWorkstream;
  reviewOpener = impl.openReview;
  reviewCloser = impl.closeReview;
  promptOpener = impl.openPrompt;
}

export function currentAppLocation(): AppLocation | null {
  return locationGetter ? locationGetter() : null;
}

export function goToWorkstream(target: WorkstreamTarget): void {
  workstreamNavigator?.(target);
}

export function openReview(target: ReviewTarget): void {
  reviewOpener?.(target);
}

export function closeReview(): void {
  reviewCloser?.();
}

export function openPrompt(prompt: string): void {
  promptOpener?.(prompt);
}
