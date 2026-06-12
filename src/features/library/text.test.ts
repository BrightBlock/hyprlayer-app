import { describe, expect, it } from "vitest";

import { parseTagsInput, splitFrontmatter } from "./text";

describe("splitFrontmatter", () => {
  it("returns the whole document when there is no frontmatter", () => {
    const doc = "# Title\n\nBody text.\n";
    expect(splitFrontmatter(doc)).toEqual({ frontmatter: null, body: doc });
  });

  it("splits a typical block", () => {
    const doc = "---\ntitle: Hello\ntags: [a, b]\n---\n# Heading\n";
    expect(splitFrontmatter(doc)).toEqual({
      frontmatter: "title: Hello\ntags: [a, b]",
      body: "# Heading\n",
    });
  });

  it("handles an empty frontmatter block", () => {
    expect(splitFrontmatter("---\n---\nbody")).toEqual({ frontmatter: "", body: "body" });
  });

  it("handles a closing delimiter at end of file without trailing newline", () => {
    expect(splitFrontmatter("---\ntitle: x\n---")).toEqual({ frontmatter: "title: x", body: "" });
  });

  it("handles CRLF line endings", () => {
    expect(splitFrontmatter("---\r\ntitle: x\r\n---\r\nbody")).toEqual({
      frontmatter: "title: x",
      body: "body",
    });
  });

  it("ignores --- delimiters that do not start the document", () => {
    const doc = "intro\n---\ntitle: x\n---\n";
    expect(splitFrontmatter(doc)).toEqual({ frontmatter: null, body: doc });
  });

  it("stops at the first closing delimiter", () => {
    expect(splitFrontmatter("---\na: 1\n---\n---\nnot fm\n")).toEqual({
      frontmatter: "a: 1",
      body: "---\nnot fm\n",
    });
  });
});

describe("parseTagsInput", () => {
  it("splits, trims, and drops empties", () => {
    expect(parseTagsInput(" a, b ,,c ")).toEqual(["a", "b", "c"]);
  });

  it("returns an empty list for blank input", () => {
    expect(parseTagsInput("")).toEqual([]);
    expect(parseTagsInput("  ,  ")).toEqual([]);
  });
});
