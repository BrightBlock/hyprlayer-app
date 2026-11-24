import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useRef } from "react";

import { useBootstrap } from "@/lib/queries";
import { log } from "@/lib/log";
import { launchPlan, pathSteps } from "./selection";
import { useCommand } from "@/lib/keybindings/registry";
import { resumeTheTour, showWhatsNew, takeTheTour } from "./actions";
import { TourHost } from "./TourHost";
import { WhatsNewCard } from "./WhatsNewCard";
import { openWhatsNewCard, setAppVersion, startTour, TOUR_STORED_AT_LAUNCH, useTourStore } from "./tourStore";

/** Decides once per launch, after setup, whether first run starts or the what's-new card opens. */
export function useTourLaunch(suspended: boolean): void {
  const bootstrap = useBootstrap().data;
  const version = useTourStore((s) => s.appVersion);
  const decided = useRef(false);

  useEffect(() => {
    void getVersion()
      .then(setAppVersion)
      .catch((e) => log.warn("tour", "reading the app version failed", e));
  }, []);

  useEffect(() => {
    if (decided.current || suspended || !version || !bootstrap || bootstrap.config.state !== "ready") return;
    decided.current = true;
    const s = useTourStore.getState();
    const plan = launchPlan({
      stored: TOUR_STORED_AT_LAUNCH,
      lastSeen: s.lastSeenVersion,
      seen: s.seen,
      hasProjects: bootstrap.projects.length > 0,
      firstRunStatus: s.firstRun.status,
    });
    const underWay = s.whatsNew.status === "active" || s.whatsNew.status === "paused";
    if (plan?.path === "firstRun") startTour("firstRun", pathSteps("firstRun"));
    else if (plan?.path === "whatsNew" && !underWay) openWhatsNewCard();
  }, [suspended, version, bootstrap]);
}

/** The app shell's one mount point for the guided tour. `suspended` while setup runs. */
export function Tour({ suspended }: { suspended: boolean }) {
  useTourLaunch(suspended);
  useCommand("help.tour", suspended ? null : takeTheTour);
  useCommand("help.whatsNew", suspended ? null : showWhatsNew);
  useCommand("help.resumeTour", suspended ? null : resumeTheTour);
  if (suspended) return null;
  return (
    <>
      <TourHost />
      <WhatsNewCard />
    </>
  );
}
