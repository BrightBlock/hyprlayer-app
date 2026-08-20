import { describe, expect, it } from "vitest";

import { formatCompactAge, formatRelativeTime, shortSha } from "./format";

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
    // A repo with no commits has no pushed_at, better blank than "just now".
    expect(formatCompactAge(null, NOW)).toBe("");
    expect(formatCompactAge(Number.NaN, NOW)).toBe("");
  });

  it("clamps a future instant to zero rather than going negative", () => {
    expect(formatCompactAge(NOW + DAY, NOW)).toBe("0m");
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
