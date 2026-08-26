import { describe, it, expect } from "vitest";
import { fuzzyScore, groupResults, flattenGroups, type SearchResult } from "./search";


describe("fuzzyScore", () => {
  it("returns null when query is not a subsequence", () => {
    expect(fuzzyScore("hello", "xyz")).toBeNull();
    expect(fuzzyScore("abc", "abd")).toBeNull();
  });

  it("returns 0 for empty query", () => {
    expect(fuzzyScore("anything", "")).toBe(0);
    expect(fuzzyScore("", "")).toBe(0);
  });

  it("accepts exact match as subsequence", () => {
    expect(fuzzyScore("hello", "hello")).not.toBeNull();
  });

  it("accepts subsequence (non-contiguous characters)", () => {
    expect(fuzzyScore("hello world", "hw")).not.toBeNull();
    expect(fuzzyScore("research-plan", "rp")).not.toBeNull();
  });

  it("is case-insensitive", () => {
    expect(fuzzyScore("Hello World", "hw")).not.toBeNull();
    expect(fuzzyScore("MY-PROJECT", "myp")).not.toBeNull();
  });

  it("prefix beats mid-word: exact prefix scores higher than mid-string match", () => {
    const prefixScore = fuzzyScore("spotlight", "spot")!;
    const midScore = fuzzyScore("xyzspotlight", "spot")!;
    expect(prefixScore).toBeGreaterThan(midScore);
  });

  it("contiguous run beats scattered: 'ab' in 'abcdef' beats 'ab' scattered in 'axbcde'", () => {
    const contiguous = fuzzyScore("abcdef", "ab")!;
    const scattered = fuzzyScore("axbcde", "ab")!;
    expect(contiguous).toBeGreaterThan(scattered);
  });

  it("word-boundary bonus: 'sp' matches better at word start", () => {
    const wordBoundary = fuzzyScore("spotlight search", "ss")!;
    const noWB = fuzzyScore("express", "ss")!;
    expect(wordBoundary).toBeGreaterThan(noWB);
  });

  it("shorter target wins ties for the same query", () => {
    const short = fuzzyScore("abc", "a")!;
    const long = fuzzyScore("abcdefghij", "a")!;
    expect(short).toBeGreaterThan(long);
  });

  it("returns a positive number for a full match", () => {
    expect(fuzzyScore("workstream", "work")).toBeGreaterThan(0);
  });
});


function makeResult(kind: SearchResult["kind"], score: number, n: number): SearchResult {
  return {
    kind,
    key: `${kind}-${n}`,
    title: `Title ${n}`,
    score,
    nav:
      kind === "doc"
        ? { type: "doc", id: `doc-${n}` }
        : kind === "workstream"
          ? { type: "workstream", projectPath: "/p", workstreamId: `ws-${n}` }
          : kind === "agent"
            ? { type: "agent", projectPath: "/p", workstreamId: `ws-${n}`, sessionId: `s-${n}` }
            : { type: "file", workstreamId: `ws-${n}`, path: `/path/${n}` },
  };
}

describe("groupResults", () => {
  it("produces groups in the order: Workstreams → Agents → Files → Documents", () => {
    const results: SearchResult[] = [
      makeResult("doc", 10, 1),
      makeResult("workstream", 20, 1),
      makeResult("agent", 15, 1),
      makeResult("file", 12, 1),
    ];
    const groups = groupResults(results);
    expect(groups.map((g) => g.kind)).toEqual(["workstream", "agent", "file", "doc"]);
  });

  it("drops empty groups", () => {
    const results: SearchResult[] = [
      makeResult("doc", 10, 1),
      makeResult("workstream", 20, 1),
    ];
    const groups = groupResults(results);
    expect(groups.map((g) => g.kind)).toEqual(["workstream", "doc"]);
  });

  it("sorts each group by score descending", () => {
    const results: SearchResult[] = [
      makeResult("doc", 5, 1),
      makeResult("doc", 20, 2),
      makeResult("doc", 10, 3),
    ];
    const groups = groupResults(results);
    const scores = groups[0].results.map((r) => r.score);
    expect(scores).toEqual([20, 10, 5]);
  });

  it("caps each group at perGroup (default 8)", () => {
    const results: SearchResult[] = Array.from({ length: 12 }, (_, i) =>
      makeResult("doc", 100 - i, i),
    );
    const groups = groupResults(results);
    expect(groups[0].results.length).toBe(8);
  });

  it("respects a custom perGroup cap", () => {
    const results: SearchResult[] = Array.from({ length: 5 }, (_, i) =>
      makeResult("doc", 100 - i, i),
    );
    const groups = groupResults(results, 3);
    expect(groups[0].results.length).toBe(3);
  });
});


describe("flattenGroups", () => {
  it("flattens groups in display order (Workstreams → Agents → Files → Documents)", () => {
    const ws = makeResult("workstream", 1, 1);
    const doc = makeResult("doc", 1, 1);
    const agent = makeResult("agent", 1, 1);
    const file = makeResult("file", 1, 1);

    const groups = groupResults([doc, ws, agent, file]);
    const flat = flattenGroups(groups);
    expect(flat.map((r) => r.kind)).toEqual(["workstream", "agent", "file", "doc"]);
  });

  it("is empty when no groups", () => {
    expect(flattenGroups([])).toEqual([]);
  });

  it("order within each group matches display order (score-sorted by groupResults)", () => {
    const results: SearchResult[] = [
      makeResult("doc", 5, 1),
      makeResult("doc", 20, 2),
      makeResult("doc", 10, 3),
    ];
    const groups = groupResults(results);
    const flat = flattenGroups(groups);
    expect(flat.map((r) => r.score)).toEqual([20, 10, 5]);
  });
});
