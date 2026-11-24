import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { flushPersistedWrites } from "@/lib/persistedStorage";
import { pathSteps } from "./selection";
import {
  activeTour,
  dismissWhatsNewCard,
  endTour,
  nextStep,
  pausedTour,
  pauseTour,
  prevStep,
  resumeTour,
  setAppVersion,
  setTaskSentAt,
  setTourWorkstream,
  startTour,
  TOUR_STORE_KEY,
  useTourStore,
} from "./tourStore";

const fresh = useTourStore.getInitialState();

beforeEach(() => {
  useTourStore.setState(fresh, true);
  setAppVersion("0.8.24");
  localStorage.clear();
});

afterEach(() => localStorage.clear());

const stored = () => {
  flushPersistedWrites();
  return JSON.parse(localStorage.getItem(TOUR_STORE_KEY) ?? "null")?.state;
};

describe("tour progress", () => {
  it("a new first run forgets the last run's task, and a new send forgets its workstream", () => {
    setTaskSentAt(1000);
    setTourWorkstream("w1");
    expect(stored()).toMatchObject({ taskSentAt: 1000, tourWorkstreamId: "w1" });
    setTaskSentAt(2000);
    expect(useTourStore.getState()).toMatchObject({ taskSentAt: 2000, tourWorkstreamId: null });
    setTourWorkstream("w2");
    startTour("whatsNew", pathSteps("whatsNew"));
    expect(useTourStore.getState().tourWorkstreamId).toBe("w2");
    startTour("firstRun", pathSteps("firstRun"));
    expect(useTourStore.getState()).toMatchObject({ taskSentAt: null, tourWorkstreamId: null });
    expect(useTourStore.getState().taskExclude).toContain("w2");
    setTaskSentAt(3000, ["live-1"]);
    expect(stored()).toMatchObject({ taskExclude: ["w2", "live-1"] });
  });

  it("walks forward and back, and finishing marks the path's features seen", () => {
    const steps = pathSteps("firstRun");
    startTour("firstRun", steps);
    expect(activeTour(useTourStore.getState())?.step.id).toBe("fr.intro");
    nextStep();
    nextStep();
    prevStep();
    expect(activeTour(useTourStore.getState())?.index).toBe(1);
    for (let i = 0; i < steps.length; i++) nextStep();
    const s = useTourStore.getState();
    expect(s.firstRun.status).toBe("done");
    expect(activeTour(s)).toBeNull();
    expect(s.seen).toEqual(expect.arrayContaining(steps.map((st) => st.feature)));
  });

  it("pauses with Esc and resumes at the same step", () => {
    startTour("firstRun", pathSteps("firstRun"));
    nextStep();
    pauseTour();
    expect(activeTour(useTourStore.getState())).toBeNull();
    expect(pausedTour(useTourStore.getState())?.step.id).toBe("fr.how");
    resumeTour();
    expect(activeTour(useTourStore.getState())?.step.id).toBe("fr.how");
  });

  it("runs one tour at a time: starting what's new pauses first run in place", () => {
    startTour("firstRun", pathSteps("firstRun"));
    nextStep();
    startTour("whatsNew", pathSteps("whatsNew"), "wn.server");
    const s = useTourStore.getState();
    expect(activeTour(s)?.step.id).toBe("wn.server");
    expect(s.firstRun).toMatchObject({ status: "paused", index: 1 });
  });

  it("Skip tour ends the path and records the version, so the card doesn't come back", () => {
    startTour("whatsNew", pathSteps("whatsNew"));
    endTour();
    const s = useTourStore.getState();
    expect(s.whatsNew.status).toBe("skipped");
    expect(s.lastSeenVersion).toBe("0.8.25");
  });

  it("dismissing the card settles the version without walking the steps", () => {
    dismissWhatsNewCard(pathSteps("whatsNew"));
    expect(useTourStore.getState().lastSeenVersion).toBe("0.8.25");
  });
});

describe("persistence", () => {
  it("writes progress to the machine-local store on every step change", () => {
    startTour("firstRun", pathSteps("firstRun"));
    nextStep();
    expect(stored()).toMatchObject({ firstRun: { status: "active", index: 1 } });
  });

  it("keeps the card's open state and the app version out of storage", () => {
    startTour("firstRun", pathSteps("firstRun"));
    const s = stored();
    expect(s).not.toHaveProperty("cardOpen");
    expect(s).not.toHaveProperty("appVersion");
  });

  it("restores a paused tour from storage", async () => {
    startTour("firstRun", pathSteps("firstRun"));
    nextStep();
    nextStep();
    pauseTour();
    const saved = stored();
    useTourStore.setState(fresh, true);
    flushPersistedWrites();
    localStorage.setItem(TOUR_STORE_KEY, JSON.stringify({ state: saved, version: 1 }));
    await useTourStore.persist.rehydrate();
    expect(pausedTour(useTourStore.getState())?.step.id).toBe("fr.open");
  });
});
