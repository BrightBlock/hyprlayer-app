// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { cardContent } from "./actions";
import { pathSteps } from "./selection";
import { TourResumeButton } from "./TourResumeButton";
import { activeTour, nextStep, openWhatsNewCard, pauseTour, setAppVersion, startTour, useTourStore } from "./tourStore";
import { WhatsNewCard } from "./WhatsNewCard";

const fresh = useTourStore.getInitialState();

beforeEach(() => {
  useTourStore.setState(fresh, true);
  setAppVersion("0.8.24");
});

afterEach(cleanup);

describe("WhatsNewCard", () => {
  it("lists what's new since the last seen version without taking focus", () => {
    const before = document.activeElement;
    act(() => openWhatsNewCard());
    render(<WhatsNewCard />);
    expect(screen.getByRole("region", { name: "Meet the Lead Engineer" })).toBeTruthy();
    expect(screen.getByText("New since 0.8.24")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: /^Show me:/ })).toHaveLength(3);
    expect(document.activeElement).toBe(before);
  });

  it("Esc inside it dismisses it and settles the version", () => {
    act(() => openWhatsNewCard());
    render(<WhatsNewCard />);
    fireEvent.keyDown(screen.getByRole("button", { name: "Later" }), { key: "Escape" });
    expect(screen.queryByRole("region")).toBeNull();
    expect(useTourStore.getState().lastSeenVersion).toBe("0.8.25");
  });

  it("Show me starts the tour at that row's step", () => {
    act(() => openWhatsNewCard());
    render(<WhatsNewCard />);
    fireEvent.click(screen.getByRole("button", { name: "Show me: Remote Control" }));
    expect(activeTour(useTourStore.getState())?.step.id).toBe("wn.remote");
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("leads with the Lead and its walkthrough, which runs the first run and settles the card", () => {
    act(() => openWhatsNewCard());
    render(<WhatsNewCard />);
    fireEvent.click(screen.getByRole("button", { name: "Take the 3-minute walkthrough" }));
    const s = useTourStore.getState();
    expect(activeTour(s)).toMatchObject({ path: "firstRun", index: 0 });
    expect(activeTour(s)?.step.id).toBe("fr.intro");
    expect(s.lastSeenVersion).toBe("0.8.25");
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("replays the newest version's steps once everything is seen", () => {
    const content = cardContent("0.8.25", []);
    expect(content.replay).toBe(true);
    expect(content.steps.map((s) => s.id)).toEqual(pathSteps("whatsNew").map((s) => s.id));
    expect(cardContent("0.8.24", ["lead.remoteControl"]).steps.map((s) => s.id)).not.toContain("wn.remote");
  });
});

describe("TourResumeButton", () => {
  it("shows only while a tour is paused, and resumes it where it was", () => {
    render(<TourResumeButton />);
    expect(screen.queryByRole("button", { name: /Resume the tour/ })).toBeNull();
    act(() => {
      startTour("firstRun", pathSteps("firstRun"));
      nextStep();
    });
    expect(screen.queryByRole("button", { name: /Resume the tour/ })).toBeNull();
    act(() => pauseTour());
    fireEvent.click(screen.getByRole("button", { name: "Resume the tour: First run, step 2 of 9" }));
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(activeTour(useTourStore.getState())?.step.id).toBe("fr.how");
  });

  it("End the tour ends it for good", () => {
    act(() => {
      startTour("firstRun", pathSteps("firstRun"));
      pauseTour();
    });
    render(<TourResumeButton />);
    fireEvent.click(screen.getByRole("button", { name: /Resume the tour/ }));
    fireEvent.click(screen.getByRole("button", { name: "End the tour" }));
    expect(useTourStore.getState().firstRun.status).toBe("skipped");
  });
});
