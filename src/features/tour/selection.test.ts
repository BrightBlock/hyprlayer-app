import { describe, expect, it } from "vitest";

import { compareVersions, launchPlan, pathSteps, seenThrough, whatsNewSteps } from "./selection";
import { PRE_TOUR_VERSION, STEPS, type TourStep } from "./steps";

const step = (id: string, since: string, feature = id): TourStep => ({
  id,
  path: "whatsNew",
  feature,
  since,
  anchor: "lead.dock",
  side: "top",
  reveal: "none",
  needs: [],
  title: id,
  body: id,
});

describe("compareVersions", () => {
  it("orders numerically, not as strings", () => {
    expect(compareVersions("0.8.10", "0.8.9")).toBe(1);
    expect(compareVersions("0.9.0", "0.10.0")).toBe(-1);
    expect(compareVersions("1.0", "1.0.0")).toBe(0);
    expect(compareVersions("0.8.25-beta.1", "0.8.25")).toBe(0);
  });
});

describe("whatsNewSteps", () => {
  const steps = [step("a", "0.8.25"), step("b", "0.9.0"), step("c", "0.8.20")];

  it("keeps only steps newer than the last seen version", () => {
    expect(whatsNewSteps("0.8.24", [], steps).map((s) => s.id)).toEqual(["a", "b"]);
    expect(whatsNewSteps("0.8.25", [], steps).map((s) => s.id)).toEqual(["b"]);
    expect(whatsNewSteps("0.9.0", [], steps)).toEqual([]);
  });

  it("drops features the user has already seen, so the first-run tour isn't repeated", () => {
    expect(whatsNewSteps("0.8.24", ["a"], steps).map((s) => s.id)).toEqual(["b"]);
  });

  it("never offers a first-run step", () => {
    const shipped = whatsNewSteps(PRE_TOUR_VERSION, []);
    expect(shipped.length).toBeGreaterThan(0);
    expect(shipped.every((s) => s.path === "whatsNew")).toBe(true);
  });
});

describe("launchPlan", () => {
  const base = { stored: true, lastSeen: null, seen: [], hasProjects: true, firstRunStatus: "new" };

  it("opens first run on a new install with no projects", () => {
    expect(launchPlan({ ...base, stored: false, hasProjects: false })).toEqual({ path: "firstRun" });
  });

  it("treats an upgrade from before the tour as having seen the last pre-tour release", () => {
    const plan = launchPlan({ ...base, stored: false });
    expect(plan?.path).toBe("whatsNew");
    expect(plan && "steps" in plan && plan.steps.map((s) => s.id)).toEqual(
      whatsNewSteps(PRE_TOUR_VERSION, []).map((s) => s.id),
    );
  });

  it("offers nothing once this version's steps are seen", () => {
    const latest = seenThrough("0.8.24", pathSteps("whatsNew"));
    expect(launchPlan({ ...base, lastSeen: latest })).toBeNull();
  });

  it("leaves a first run that is under way alone", () => {
    expect(launchPlan({ ...base, firstRunStatus: "paused" })).toBeNull();
  });
});

describe("steps", () => {
  it("keeps each path to the length the design sets", () => {
    expect(pathSteps("firstRun")).toHaveLength(6);
    expect(pathSteps("whatsNew")).toHaveLength(5);
  });

  it("gives every step a unique id and a TRY IT a waiting and a done line", () => {
    expect(new Set(STEPS.map((s) => s.id)).size).toBe(STEPS.length);
    for (const s of STEPS) if (s.tryIt) expect(s.tryIt.waiting && s.tryIt.done).toBeTruthy();
  });

  it("asks the first task to read only and archive its workstream", () => {
    const prompt = STEPS.find((s) => s.id === "fr.prompt")?.tryIt?.prompt ?? "";
    expect(prompt).toMatch(/Read only: no edits, commits or PRs\./);
    expect(prompt).toMatch(/Archive its workstream/);
  });
});

describe("seenThrough", () => {
  it("records the newest of the app version and the shown steps", () => {
    expect(seenThrough("0.8.24", [step("a", "0.8.25")])).toBe("0.8.25");
    expect(seenThrough("0.9.1", [step("a", "0.8.25")])).toBe("0.9.1");
  });
});
