
export type AppLocation = {
  pathname: string;
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

export function currentAppLocation(): AppLocation | null {
  return locationGetter ? locationGetter() : null;
}

export function goToWorkstream(target: WorkstreamTarget): void {
  workstreamNavigator?.(target);
}
