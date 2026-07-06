import { describe, expect, it } from "vitest";

import type { Project } from "@/lib/bindings";
import {
  AUTO_SYNC_COOLDOWN_MS,
  resolveSyncProjectPath,
  shouldAutoSync,
} from "./thoughtsSync";

const project = (path: string, thoughtsProject: string) =>
  ({ path, thoughtsProject }) as Project;

const PROJECTS = [
  project("C:\\repos\\alpha", "alpha"),
  project("C:\\repos\\beta", "beta_key"),
];

describe("resolveSyncProjectPath", () => {
  it("prefers the repo the Library was opened from", () => {
    expect(
      resolveSyncProjectPath({ from: "C:\\repos\\beta", project: "alpha" }, null, PROJECTS),
    ).toBe("C:\\repos\\beta");
  });

  it("falls back to the scope key when `from` isn't a known project", () => {
    expect(
      resolveSyncProjectPath({ from: "C:\\elsewhere", project: "beta_key" }, null, PROJECTS),
    ).toBe("C:\\repos\\beta");
  });

  it("resolves the scoped project by thoughts key", () => {
    expect(resolveSyncProjectPath({ project: "beta_key" }, null, PROJECTS)).toBe(
      "C:\\repos\\beta",
    );
  });

  it("uses the last-used scope when the Library is unscoped", () => {
    expect(resolveSyncProjectPath({}, "beta_key", PROJECTS)).toBe("C:\\repos\\beta");
  });

  it("falls back to the first project when nothing is scoped", () => {
    expect(resolveSyncProjectPath({}, null, PROJECTS)).toBe("C:\\repos\\alpha");
  });

  it("returns undefined without projects", () => {
    expect(resolveSyncProjectPath({ project: "alpha" }, "alpha", [])).toBeUndefined();
  });

  it("falls back to the first project on a stale scope key", () => {
    expect(resolveSyncProjectPath({ project: "gone" }, null, PROJECTS)).toBe(
      "C:\\repos\\alpha",
    );
  });
});

describe("shouldAutoSync", () => {
  it("allows the first sync of a run (epoch lastAttempt)", () => {
    expect(shouldAutoSync(1_000_000, 0)).toBe(true);
  });

  it("blocks re-syncs inside the cooldown", () => {
    expect(shouldAutoSync(10_000 + AUTO_SYNC_COOLDOWN_MS - 1, 10_000)).toBe(false);
  });

  it("allows a sync once the cooldown has elapsed", () => {
    expect(shouldAutoSync(10_000 + AUTO_SYNC_COOLDOWN_MS, 10_000)).toBe(true);
  });
});
