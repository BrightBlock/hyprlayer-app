import { PRE_TOUR_VERSION, STEPS, type TourPath, type TourStep } from "./steps";

/** Numeric dotted compare; a pre-release or build suffix is ignored. */
export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.split(/[-+]/)[0].split(".").map((n) => Number.parseInt(n, 10) || 0);
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return Math.sign(d);
  }
  return 0;
}

export function maxVersion(versions: readonly string[]): string | null {
  return versions.reduce<string | null>((m, v) => (m === null || compareVersions(v, m) > 0 ? v : m), null);
}

export function pathSteps(path: TourPath, steps: readonly TourStep[] = STEPS): TourStep[] {
  return steps.filter((s) => s.path === path);
}

/**
 * What's new since `lastSeen`: steps that shipped after it, minus features already seen. The build's own
 * steps are by definition in this build, so there is no upper bound on `since`.
 */
export function whatsNewSteps(
  lastSeen: string,
  seen: readonly string[],
  steps: readonly TourStep[] = STEPS,
): TourStep[] {
  return pathSteps("whatsNew", steps).filter(
    (s) => compareVersions(s.since, lastSeen) > 0 && !seen.includes(s.feature),
  );
}

export type Launch = { path: "firstRun" } | { path: "whatsNew"; steps: TourStep[] } | null;

/**
 * Decides what opens on its own at launch. `stored` is whether a tour store existed before this launch; an
 * upgrade from before the tour has projects but no store, and counts as having seen PRE_TOUR_VERSION.
 */
export function launchPlan({
  stored,
  lastSeen,
  seen,
  hasProjects,
  firstRunStatus,
}: {
  stored: boolean;
  lastSeen: string | null;
  seen: readonly string[];
  hasProjects: boolean;
  firstRunStatus: string;
}): Launch {
  if (!stored && !hasProjects) return { path: "firstRun" };
  if (firstRunStatus === "active" || firstRunStatus === "paused") return null;
  const steps = whatsNewSteps(lastSeen ?? PRE_TOUR_VERSION, seen);
  return steps.length ? { path: "whatsNew", steps } : null;
}

/** The version a finished what's-new run records, so the same steps don't come back on the next launch. */
export function seenThrough(current: string, shown: readonly TourStep[]): string {
  return maxVersion([current, ...shown.map((s) => s.since)]) ?? current;
}
