/**
 * Indirection between app wiring and the shared layers below it: the router
 * singleton lives in src/router.tsx (wiring), but lib/feature modules
 * occasionally need "where is the user?" (notification suppression) or "take
 * the user to this workstream" (notification click-through). They call these
 * functions; router.tsx registers the real implementations at import time, so
 * no feature or lib module imports the router and the import direction stays
 * routes → features → lib (see docs/adr/0003-frontend-layering.md).
 */

export type AppLocation = {
  pathname: string;
  /** Parsed search params of the current route (shape depends on the route). */
  search: Record<string, unknown>;
};

type WorkstreamTarget = { projectPath: string; workstreamId: string };

let locationGetter: (() => AppLocation) | null = null;
let workstreamNavigator: ((target: WorkstreamTarget) => void) | null = null;

export function registerAppNav(impl: {
  currentLocation: () => AppLocation;
  goToWorkstream: (target: WorkstreamTarget) => void;
}): void {
  locationGetter = impl.currentLocation;
  workstreamNavigator = impl.goToWorkstream;
}

/** Null until router.tsx registers — i.e. in jsdom tests, which never wire it. */
export function currentAppLocation(): AppLocation | null {
  return locationGetter ? locationGetter() : null;
}

export function goToWorkstream(target: WorkstreamTarget): void {
  workstreamNavigator?.(target);
}
