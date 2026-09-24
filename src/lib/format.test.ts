import { describe, expect, it } from "vitest";

import {
  formatCompactAge,
  formatCompactCount,
  formatRelativeTime,
  formatTokens,
  shortSha,
} from "./format";

const NOW = Date.UTC(2026, 6, 10, 12, 0, 0);
const ago = (ms: number) => formatCompactAge(NOW - ms, NOW);

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe("formatCompactAge", () => {
  it("steps up through each unit at its boundary", () => {
    expect(ago(30_000)).toBe("0m");
    expect(ago(59 * MIN)).toBe("59m");
    expect(ago(HOUR)).toBe("1h");
    expect(ago(23 * HOUR)).toBe("23h");
    expect(ago(DAY)).toBe("1d");
    expect(ago(6 * DAY)).toBe("6d");
    expect(ago(7 * DAY)).toBe("1w");
    expect(ago(34 * DAY)).toBe("4w");
    expect(ago(35 * DAY)).toBe("1mo");
    expect(ago(365 * DAY)).toBe("1y");
  });

  it("does not floor the last months of the first year into '0y'", () => {
    expect(ago(364 * DAY)).toBe("12mo");
    expect(formatRelativeTime(NOW - 364 * DAY, NOW)).toBe("12mo ago");
  });

  it("renders nothing for a missing or invalid instant", () => {
    expect(formatCompactAge(null, NOW)).toBe("");
    expect(formatCompactAge(Number.NaN, NOW)).toBe("");
  });

  it("clamps a future instant to zero rather than going negative", () => {
    expect(formatCompactAge(NOW + DAY, NOW)).toBe("0m");
  });
});

describe("formatCompactCount", () => {
  it("prints counts under a thousand as they are", () => {
    expect(formatCompactCount(0)).toBe("0");
    expect(formatCompactCount(999)).toBe("999");
  });

  it("drops a zero tenth rather than printing '1.0k'", () => {
    expect(formatCompactCount(1000)).toBe("1k");
    expect(formatCompactCount(1050)).toBe("1k");
    expect(formatCompactCount(1_000_000)).toBe("1M");
  });

  it("floors the tenth, never rounding a count up past what it is", () => {
    expect(formatCompactCount(3400)).toBe("3.4k");
    expect(formatCompactCount(3499)).toBe("3.4k");
    expect(formatCompactCount(9999)).toBe("9.9k");
    expect(formatCompactCount(999_999)).toBe("999k");
    expect(formatCompactCount(1_250_000)).toBe("1.2M");
  });

  it("drops the decimal from ten units up", () => {
    expect(formatCompactCount(10_000)).toBe("10k");
    expect(formatCompactCount(11_000)).toBe("11k");
    expect(formatCompactCount(11_999)).toBe("11k");
    expect(formatCompactCount(12_500_000)).toBe("12M");
  });

  it("floors fractional input before scaling", () => {
    expect(formatCompactCount(1234.9)).toBe("1.2k");
    expect(formatCompactCount(999.9)).toBe("999");
  });

  it("reads 0 for input that is not a count", () => {
    expect(formatCompactCount(Number.NaN)).toBe("0");
    expect(formatCompactCount(Number.POSITIVE_INFINITY)).toBe("0");
    expect(formatCompactCount(-5)).toBe("0");
  });

  it("carries no sign; the caller adds it", () => {
    expect(formatCompactCount(3400)).not.toMatch(/[+\-−]/);
  });
});

describe("shortSha", () => {
  it("truncates a full sha to seven characters", () => {
    expect(shortSha("a1b2c3d4e5f6789012345678901234567890abcd")).toBe("a1b2c3d");
  });

  it("returns an already-short input unchanged", () => {
    expect(shortSha("a1b2c3")).toBe("a1b2c3");
  });
});

describe("formatTokens", () => {
  it("scales to k and M", () => {
    expect(formatTokens(980)).toBe("980");
    expect(formatTokens(24130)).toBe("24.1k");
    expect(formatTokens(269_500)).toBe("269.5k");
    expect(formatTokens(1_000_000)).toBe("1.0M");
  });
});
