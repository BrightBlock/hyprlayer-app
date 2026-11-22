import { describe, expect, it } from "vitest";

import { nextDockPhase, type DockEvent, type DockPhase } from "./dockPhase";

const run = (from: DockPhase, events: DockEvent[]) => events.reduce(nextDockPhase, from);

describe("nextDockPhase", () => {
  it("expands through a grow, then minimizes back to collapsed", () => {
    expect(run("closed", ["expand", "settled"])).toBe("open");
    expect(run("open", ["minimize", "settled", "settled"])).toBe("closed");
  });

  it("collapses when minimized mid-grow", () => {
    expect(run("closed", ["expand", "minimize", "settled", "settled"])).toBe("closed");
  });

  it("does not reopen while the minimize is still running", () => {
    expect(run("open", ["minimize", "restored"])).toBe("fading");
    expect(run("open", ["minimize", "settled", "restored", "expand"])).toBe("shrinking");
  });

  it("opens without a grow when the remembered state restores it", () => {
    expect(run("closed", ["restored"])).toBe("open");
  });

  it("closes at once when the remembered state is dropped", () => {
    expect(run("open", ["dropped"])).toBe("closed");
  });
});
