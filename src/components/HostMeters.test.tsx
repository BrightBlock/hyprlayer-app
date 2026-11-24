// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { HostStats } from "@/lib/bindings";
import { cacheBytes, formatPercent, HostMeters, meterTone, ofTotal } from "./HostMeters";

const GB = 1e9;
const STATS: HostStats = {
  load: [9.6, 8, 7],
  cpus: 32,
  memoryUsed: 12 * GB,
  memoryTotal: 64 * GB,
  memoryFree: 30 * GB,
  swapUsed: 0.4 * GB,
  swapTotal: 125 * GB,
  cpuPercent: 31,
  cpuHistory: [10, 75, 95],
};

afterEach(cleanup);

describe("meterTone", () => {
  it.each([
    [0, "ok"],
    [69.9, "ok"],
    [70, "warn"],
    [89.9, "warn"],
    [90, "bad"],
    [100, "bad"],
  ] as const)("%s%% is %s", (percent, tone) => {
    expect(meterTone(percent)).toBe(tone);
  });
});

describe("formatPercent", () => {
  it("shows a trace as under one percent", () => {
    expect(formatPercent(0)).toBe("0%");
    expect(formatPercent(0.3)).toBe("<1%");
    expect(formatPercent(31.4)).toBe("31%");
  });
});

describe("cacheBytes", () => {
  it("is what is neither used nor free", () => {
    expect(cacheBytes(STATS)).toBe(22 * GB);
  });
  it("is hidden when the host reports no free figure", () => {
    expect(cacheBytes({ ...STATS, memoryFree: null })).toBeNull();
    expect(cacheBytes({ ...STATS, memoryFree: undefined })).toBeNull();
  });
  it("never goes negative", () => {
    expect(cacheBytes({ ...STATS, memoryFree: 60 * GB })).toBe(0);
  });
});

describe("ofTotal", () => {
  it("shares a unit when both sides use it", () => {
    expect(ofTotal(12 * GB, 64 * GB)).toBe("12.0 of 64.0 GB");
    expect(ofTotal(433e6, 125 * GB)).toBe("433 MB of 125.0 GB");
  });
});

describe("HostMeters", () => {
  it("draws CPU, memory with cache, and swap meters", () => {
    render(<HostMeters stats={STATS} cpuPercent={31} />);
    expect(screen.getByRole("meter", { name: "CPU usage" }).getAttribute("aria-valuenow")).toBe("31");
    expect(screen.getByRole("meter", { name: "Memory used" }).getAttribute("aria-valuenow")).toBe("19");
    expect(screen.getByRole("meter", { name: "Swap used" })).toBeTruthy();
    expect(screen.getByText("load 0.30/core").getAttribute("title")).toMatch(/Under 1.0 means nothing is waiting/);
    expect(screen.getByText(/22.0 GB cache/)).toBeTruthy();
    expect(screen.getByTestId("cache-segment").style.transform).toBe(`scaleX(${34 / 64})`);
  });

  it("colours the percent at the thresholds", () => {
    render(<HostMeters stats={STATS} cpuPercent={94} />);
    expect(screen.getByText("94%").className).toContain("text-meter-bad-ink");
    cleanup();
    render(<HostMeters stats={STATS} cpuPercent={72} />);
    expect(screen.getByText("72%").className).toContain("text-meter-warn-ink");
  });

  it("colours sparkline columns from the meter's own scale, not the theme's ok", () => {
    render(<HostMeters stats={STATS} cpuPercent={31} />);
    const columns = [...screen.getByRole("img", { name: /CPU usage over/ }).children].slice(-3);
    expect(columns.map((c) => c.className.match(/bg-meter-\w+/)?.[0])).toEqual([
      "bg-meter-ok",
      "bg-meter-warn",
      "bg-meter-bad",
    ]);
  });

  it("says a host has no swap and draws no bar for it", () => {
    render(<HostMeters stats={{ ...STATS, swapUsed: 0, swapTotal: 0 }} cpuPercent={12} />);
    expect(screen.getByText("none on this host")).toBeTruthy();
    expect(screen.queryByRole("meter", { name: "Swap used" })).toBeNull();
  });

  it("leaves the cache out where the host has no free figure", () => {
    render(<HostMeters stats={{ ...STATS, memoryFree: null }} cpuPercent={12} />);
    expect(screen.queryByTestId("cache-segment")).toBeNull();
    expect(screen.queryByText(/cache/)).toBeNull();
  });
});
