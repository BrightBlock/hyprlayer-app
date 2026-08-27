import { describe, expect, it } from "vitest";

const TSX_SOURCES = import.meta.glob<string>("../**/*.tsx", {
  eager: true,
  import: "default",
  query: "?raw",
});

describe("sidebar visibility policy", () => {
  it("keeps every sidebar shell free of reveal transitions", () => {
    const sidebars: string[] = [];
    const animated: string[] = [];

    for (const [file, source] of Object.entries(TSX_SOURCES)) {
      for (const match of source.matchAll(/<aside\b[\s\S]*?>/g)) {
        const line = source.slice(0, match.index).split("\n").length;
        const location = `${file.replace(/^\.\.\//, "")}:${line}`;
        sidebars.push(location);
        if (/\btransition(?:\b|Property\b)/.test(match[0])) animated.push(location);
      }
    }

    expect(sidebars.length).toBeGreaterThan(0);
    expect(animated).toEqual([]);
  });
});
