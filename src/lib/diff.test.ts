import { describe, expect, it } from "vitest";

import { diffStrings, intraLineRanges } from "./diff";

describe("diffStrings", () => {
  it("keeps unchanged lines as context and prefixes them with a space", () => {
    const lines = diffStrings("a\nb\nc", "a\nb\nc");
    expect(lines.map((l) => l.kind)).toEqual(["context", "context", "context"]);
    expect(lines.map((l) => l.text)).toEqual([" a", " b", " c"]);
  });

  it("marks a single changed line as a del followed by an add", () => {
    const lines = diffStrings("a\nold\nc", "a\nnew\nc");
    expect(lines).toEqual([
      { kind: "context", text: " a" },
      { kind: "del", text: "-old" },
      { kind: "add", text: "+new" },
      { kind: "context", text: " c" },
    ]);
  });

  it("reports pure insertions and deletions", () => {
    expect(diffStrings("", "x")).toEqual([
      { kind: "del", text: "-" },
      { kind: "add", text: "+x" },
    ]);
    expect(diffStrings("a\nb", "a")).toEqual([
      { kind: "context", text: " a" },
      { kind: "del", text: "-b" },
    ]);
  });
});

describe("intraLineRanges", () => {
  const doc = "aaa\nbbbb\nccc";

  it("keeps a mid-line slice (a word changed inside a modified line)", () => {
    expect(intraLineRanges(doc, 5, 7)).toEqual([[5, 7]]);
  });

  it("keeps a slice that runs to the end of a line", () => {
    expect(intraLineRanges(doc, 6, 8)).toEqual([[6, 8]]);
  });

  it("drops a span that covers a whole line (a wholly-new/removed line)", () => {
    expect(intraLineRanges(doc, 4, 8)).toEqual([]);
  });

  it("drops every whole line inside a multi-line span", () => {
    expect(intraLineRanges(doc, 0, doc.length)).toEqual([]);
  });

  it("keeps only the partial edges of a span crossing several lines", () => {
    expect(intraLineRanges(doc, 1, 11)).toEqual([
      [1, 3],
      [9, 11],
    ]);
  });

  it("ignores an empty span", () => {
    expect(intraLineRanges(doc, 5, 5)).toEqual([]);
  });
});
