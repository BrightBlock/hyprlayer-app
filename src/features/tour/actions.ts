import { compareVersions, maxVersion, pathSteps, whatsNewSteps } from "./selection";
import { PRE_TOUR_VERSION, type TourStep } from "./steps";
import { dismissWhatsNewCard, openWhatsNewCard, resumeTour, startTour, useTourStore } from "./tourStore";

/** Help › Take the tour: first run, always from step 1. */
export function takeTheTour(): void {
  startTour("firstRun", pathSteps("firstRun"));
}

/** The what's-new card's walkthrough: the first run, which settles the card's version too. */
export function takeTheWalkthrough(shown: readonly TourStep[]): void {
  dismissWhatsNewCard(shown);
  takeTheTour();
}

/** Help › What's new: the card, replaying the newest version's steps once nothing is new. */
export function showWhatsNew(): void {
  openWhatsNewCard();
}

export function resumeTheTour(): void {
  resumeTour();
}

export type CardContent = { steps: TourStep[]; since: string; replay: boolean };

export function cardContent(lastSeen: string | null, seen: readonly string[]): CardContent {
  const since = lastSeen ?? PRE_TOUR_VERSION;
  const fresh = whatsNewSteps(since, seen);
  if (fresh.length) return { steps: fresh, since, replay: false };
  const all = pathSteps("whatsNew");
  const latest = maxVersion(all.map((s) => s.since)) ?? since;
  return { steps: all.filter((s) => compareVersions(s.since, latest) === 0), since: latest, replay: true };
}

export function useCardContent(): CardContent {
  const lastSeen = useTourStore((s) => s.lastSeenVersion);
  const seen = useTourStore((s) => s.seen);
  return cardContent(lastSeen, seen);
}
