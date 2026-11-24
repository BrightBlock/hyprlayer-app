import { create } from "zustand";
import { persist } from "zustand/middleware";

import { debouncedJSONStorage, flushPersistedWrites } from "@/lib/persistedStorage";
import { seenThrough } from "./selection";
import { STEPS, type TourPath, type TourStep } from "./steps";

export const TOUR_STORE_KEY = "hyprlayer:tour";

export type TourStatus = "new" | "active" | "paused" | "done" | "skipped";

type Progress = { status: TourStatus; stepIds: string[]; index: number };

type Persisted = {
  lastSeenVersion: string | null;
  /** Features whose step was finished or skipped. */
  seen: string[];
  firstRun: Progress;
  whatsNew: Progress;
  /** The project and the workstream the first-run task made, so later steps find them. */
  projectPath: string | null;
  tourWorkstreamId: string | null;
};

type TourState = Persisted & {
  /** Not persisted: the launch decides it again. */
  cardOpen: boolean;
  appVersion: string;
};

const IDLE: Progress = { status: "new", stepIds: [], index: 0 };

function storedBeforeLaunch(): boolean {
  try {
    return typeof localStorage !== "undefined" && localStorage.getItem(TOUR_STORE_KEY) !== null;
  } catch {
    return false;
  }
}

/** Read once at module load, before persist writes the first copy. */
export const TOUR_STORED_AT_LAUNCH = storedBeforeLaunch();

const initial = (): TourState => ({
  lastSeenVersion: null,
  seen: [],
  firstRun: IDLE,
  whatsNew: IDLE,
  projectPath: null,
  tourWorkstreamId: null,
  cardOpen: false,
  appVersion: "",
});

export const useTourStore = create<TourState>()(
  persist(initial, {
    name: TOUR_STORE_KEY,
    version: 1,
    storage: debouncedJSONStorage<Persisted>(),
    partialize: ({ lastSeenVersion, seen, firstRun, whatsNew, projectPath, tourWorkstreamId }): Persisted => ({
      lastSeenVersion,
      seen,
      firstRun,
      whatsNew,
      projectPath,
      tourWorkstreamId,
    }),
  }),
);

const byId = new Map(STEPS.map((s) => [s.id, s]));

export function stepsOf(progress: Progress): TourStep[] {
  return progress.stepIds.flatMap((id) => byId.get(id) ?? []);
}

export type ActiveTour = { path: TourPath; steps: TourStep[]; index: number; step: TourStep };

export function activeTour(s: Pick<TourState, "firstRun" | "whatsNew">): ActiveTour | null {
  for (const path of ["firstRun", "whatsNew"] as const) {
    const p = s[path];
    if (p.status !== "active") continue;
    const steps = stepsOf(p);
    const step = steps[p.index];
    if (step) return { path, steps, index: p.index, step };
  }
  return null;
}

export function pausedTour(s: Pick<TourState, "firstRun" | "whatsNew">): ActiveTour | null {
  for (const path of ["firstRun", "whatsNew"] as const) {
    const p = s[path];
    if (p.status !== "paused") continue;
    const steps = stepsOf(p);
    const step = steps[p.index];
    if (step) return { path, steps, index: p.index, step };
  }
  return null;
}

const set = useTourStore.setState;
const get = useTourStore.getState;

function other(path: TourPath): TourPath {
  return path === "firstRun" ? "whatsNew" : "firstRun";
}

function persistNow() {
  // A step change must survive a crash or a dev reinstall a second later.
  flushPersistedWrites();
}

export function setAppVersion(version: string): void {
  set({ appVersion: version });
}

export function startTour(path: TourPath, steps: readonly TourStep[], startAt?: string): void {
  if (!steps.length) return;
  const index = Math.max(0, startAt ? steps.findIndex((s) => s.id === startAt) : 0);
  const o = other(path);
  set((s) => ({
    [path]: { status: "active", stepIds: steps.map((st) => st.id), index },
    // One tour at a time: the other one waits, paused where it was.
    [o]: s[o].status === "active" ? { ...s[o], status: "paused" } : s[o],
    cardOpen: false,
  }));
  persistNow();
}

function finish(path: TourPath, status: "done" | "skipped") {
  set((s) => {
    const steps = stepsOf(s[path]);
    const seen = [...new Set([...s.seen, ...steps.map((st) => st.feature)])];
    return {
      [path]: { ...s[path], status },
      seen,
      lastSeenVersion: path === "whatsNew" ? seenThrough(s.appVersion || "0.0.0", steps) : s.lastSeenVersion,
    };
  });
  persistNow();
}

function move(path: TourPath, delta: number, markSeen: boolean) {
  const s = get();
  const p = s[path];
  const steps = stepsOf(p);
  const current = steps[p.index];
  const seen = markSeen && current ? [...new Set([...s.seen, current.feature])] : s.seen;
  const index = p.index + delta;
  if (index >= steps.length) {
    set({ seen });
    finish(path, "done");
    return;
  }
  set({ [path]: { ...p, index: Math.max(0, index) }, seen });
  persistNow();
}

export function nextStep(): void {
  const t = activeTour(get());
  if (t) move(t.path, 1, true);
}

export function prevStep(): void {
  const t = activeTour(get());
  if (t) move(t.path, -1, false);
}

export function skipStep(): void {
  nextStep();
}

/** Esc, the ✕, or the anchor leaving: the place is kept for Resume. */
export function pauseTour(): void {
  const t = activeTour(get());
  if (!t) return;
  set((s) => ({ [t.path]: { ...s[t.path], status: "paused" } }));
  persistNow();
}

export function resumeTour(): void {
  const t = pausedTour(get());
  if (!t) return;
  const o = other(t.path);
  set((s) => ({
    [t.path]: { ...s[t.path], status: "active" },
    [o]: s[o].status === "active" ? { ...s[o], status: "paused" } : s[o],
  }));
  persistNow();
}

/** Skip tour: the path ends and its features count as seen. */
export function endTour(path?: TourPath): void {
  const t = path ? { path } : (activeTour(get()) ?? pausedTour(get()));
  if (t) finish(t.path, "skipped");
}

export function openWhatsNewCard(): void {
  set({ cardOpen: true });
}

/** Later and ✕ on the card both settle this version: the card won't come back on its own. */
export function dismissWhatsNewCard(shown: readonly TourStep[]): void {
  set((s) => ({
    cardOpen: false,
    lastSeenVersion: seenThrough(s.appVersion || "0.0.0", shown),
  }));
  persistNow();
}

export function setTourProject(projectPath: string): void {
  set({ projectPath });
}

export function setTourWorkstream(workstreamId: string | null): void {
  set({ tourWorkstreamId: workstreamId });
}
