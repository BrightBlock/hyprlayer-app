import { describe, expect, it } from "vitest";

import type { Session } from "@/lib/bindings";
import { IDLE_MS, isReleasePair, isServerIdle, type IdleInputs } from "./serverAutoUpdate";

const session = (patch: Partial<Session>): Session => ({
  id: "s",
  workstreamId: "w",
  stage: "implement",
  harness: "claude",
  claudeSessionId: null,
  cwd: "/tmp",
  state: "running",
  model: null,
  costUsd: null,
  startedAt: null,
  endedAt: null,
  exitSummary: null,
  working: false,
  ...patch,
});

const QUIET: IdleInputs = { sessions: [], runningScripts: 0, agents: [], quietMs: IDLE_MS };

describe("isServerIdle", () => {
  it("is idle with nothing running and no recent input", () => {
    expect(isServerIdle(QUIET)).toBe(true);
  });

  it("treats a live harness between turns as idle", () => {
    expect(isServerIdle({ ...QUIET, sessions: [session({ state: "running", working: false })] })).toBe(true);
  });

  it("is busy while a session is mid-turn", () => {
    expect(isServerIdle({ ...QUIET, sessions: [session({ working: true })] })).toBe(false);
  });

  it.each(["starting", "awaiting_permission", "awaiting_question"] as const)(
    "is busy while a session is %s",
    (state) => {
      expect(isServerIdle({ ...QUIET, sessions: [session({ state, working: false })] })).toBe(false);
    },
  );

  it("is busy while a script runs", () => {
    expect(isServerIdle({ ...QUIET, runningScripts: 1 })).toBe(false);
  });

  it.each(["working", "blocked"] as const)("is busy while a terminal agent is %s", (state) => {
    expect(isServerIdle({ ...QUIET, agents: ["idle", state] })).toBe(false);
  });

  it("is busy while the user has touched the app recently", () => {
    expect(isServerIdle({ ...QUIET, quietMs: IDLE_MS - 1 })).toBe(false);
  });
});

describe("isReleasePair", () => {
  it("allows a release app over a release server", () => {
    expect(isReleasePair("0.8.6", "0.40.0")).toBe(true);
  });

  it("never replaces a dev-installed server", () => {
    expect(isReleasePair("0.8.7+dev.abc123", "0.40.0")).toBe(false);
  });

  it("never pushes a dev-installed app's server", () => {
    expect(isReleasePair("0.8.6", "0.40.0+dev.abc123")).toBe(false);
  });

  it("holds off until both versions are known", () => {
    expect(isReleasePair(null, "0.40.0")).toBe(false);
    expect(isReleasePair("0.8.6", "")).toBe(false);
  });
});
