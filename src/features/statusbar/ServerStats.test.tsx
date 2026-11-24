// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { HostStats } from "@/lib/bindings";
import { ServerStatsReadout, serverStatsSummary, serverStatsView } from "./ServerStats";

const CALM: HostStats = {
  load: [9.6, 9, 8.5],
  cpus: 32,
  memoryUsed: 12e9,
  memoryTotal: 60e9,
  swapUsed: 0.4e9,
  swapTotal: 125e9,
  cpuPercent: 31,
  cpuHistory: [22, 25, 31, 28, 24, 19, 21, 26, 34, 31],
  memoryFree: 20e9,
};

const HOT: HostStats = {
  ...CALM,
  load: [47.4, 45, 40],
  memoryUsed: 57e9,
  swapUsed: 41e9,
  cpuPercent: 94,
  cpuHistory: [60, 72, 91, 94, 98, 94],
};

afterEach(cleanup);

function renderReadout(stats: HostStats | undefined, compact = false) {
  render(<ServerStatsReadout view={serverStatsView(stats, 4, 9)} compact={compact} />);
}

describe("serverStatsView", () => {
  it("keeps the last six CPU samples, padding a short history on the left", () => {
    expect(serverStatsView(CALM, 4, 9).cpu).toEqual({ kind: "percent", percent: 31, recent: [24, 19, 21, 26, 34, 31] });
    const fresh = serverStatsView({ ...CALM, cpuHistory: [12, 14] }, 4, 9).cpu;
    expect(fresh).toEqual({ kind: "percent", percent: 31, recent: [null, null, null, null, 12, 14] });
  });

  it("shows swap only once it reaches 10%", () => {
    expect(serverStatsView(CALM, 4, 9).swap).toBeNull();
    expect(Math.round(serverStatsView(HOT, 4, 9).swap!)).toBe(33);
    expect(serverStatsView({ ...HOT, swapTotal: 0 }, 4, 9).swap).toBeNull();
  });
});

describe("ServerStatsReadout states", () => {
  it("waits for a second sample before drawing CPU, while memory shows from the first", () => {
    renderReadout({ ...CALM, cpuPercent: null, cpuHistory: [] });
    expect(screen.getByTestId("stats-cpu-pending")).toBeTruthy();
    expect(screen.getByTestId("stats-memory").dataset.percent).toBe("20");
    expect(serverStatsSummary(serverStatsView({ ...CALM, cpuPercent: null, cpuHistory: [] }, 4, 9))).toBe(
      "Memory 20% · 4 agents running · 9 workstreams",
    );
  });

  it("draws empty graphs before any stats arrive", () => {
    renderReadout(undefined);
    expect(screen.getByTestId("stats-cpu-pending")).toBeTruthy();
    expect(screen.getByTestId("stats-memory").dataset.percent).toBeUndefined();
  });

  it("stands in load per core for CPU on an older server that sends no history", () => {
    const old: HostStats = { ...CALM, cpuPercent: null, cpuHistory: null, memoryFree: null };
    renderReadout(old);
    expect(screen.getByTestId("stats-load").textContent).toBe("0.30");
    expect(screen.queryByTestId("stats-cpu")).toBeNull();
    expect(serverStatsSummary(serverStatsView(old, 4, 9))).toBe(
      "load 0.30/core · Memory 20% · 4 agents running · 9 workstreams",
    );
  });

  it("colours the hot CPU samples and adds swap under high load, still with no percents", () => {
    renderReadout(HOT);
    const cpu = screen.getByTestId("stats-cpu");
    const tones = [...cpu.children].map((bar) => (bar as HTMLElement).dataset.tone);
    expect(tones).toEqual(["ok", "warn", "bad", "bad", "bad", "bad"]);
    expect(screen.getByTestId("stats-memory").dataset.percent).toBe("95");
    expect(screen.getByTestId("stats-swap").dataset.percent).toBe("33");
    expect(document.body.textContent).not.toContain("%");
    expect(serverStatsSummary(serverStatsView(HOT, 6, 9))).toBe(
      "CPU 94% · Memory 95% · Swap 33% · 6 agents running · 9 workstreams",
    );
  });

  it("drops the counts at compact width", () => {
    renderReadout(CALM, true);
    expect(screen.getByTestId("stats-cpu")).toBeTruthy();
    expect(screen.getByTestId("stats-memory")).toBeTruthy();
    expect(screen.queryByTestId("stats-agents")).toBeNull();
    expect(screen.queryByTestId("stats-workstreams")).toBeNull();
  });

  it("says one agent, not one agents", () => {
    expect(serverStatsSummary(serverStatsView(CALM, 1, 1))).toContain("1 agent running · 1 workstream");
  });
});
