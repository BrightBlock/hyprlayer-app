import { describe, expect, it } from "vitest";

import { serversHeadline, serversMeta, workshopMeta } from "./settingsMeta";

describe("workshopMeta", () => {
  it("counts agents and skills", () => {
    expect(workshopMeta(6, 14)).toBe("6 agents · 14 skills");
    expect(workshopMeta(0, 0)).toBe("0 agents · 0 skills");
  });

  it("uses the singular for one", () => {
    expect(workshopMeta(1, 1)).toBe("1 agent · 1 skill");
  });

  it("says nothing numeric until both counts load, never a fake zero", () => {
    expect(workshopMeta(null, 14)).toBe("—");
    expect(workshopMeta(6, null)).toBe("—");
  });
});

describe("server labels", () => {
  it("lead with the total and add offline hosts only when there are any", () => {
    expect(serversMeta({ total: 3, unreachable: 1 })).toBe("3 · 1 offline");
    expect(serversMeta({ total: 1, unreachable: 0 })).toBe("1");
  });

  it("spell out the headline, singular for one", () => {
    expect(serversHeadline({ total: 3, unreachable: 1 })).toBe("3 servers · 1 unreachable");
    expect(serversHeadline({ total: 1, unreachable: 0 })).toBe("1 server");
  });
});
