import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { ensureSyntaxTree } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { extractHeadings } from "./headings";

function mkState(doc: string): EditorState {
  const state = EditorState.create({
    doc,
    extensions: [markdown({ base: markdownLanguage, codeLanguages: languages })],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  return state;
}

describe("extractHeadings", () => {
  // Failure mode 1: an H1 is included, or an H5 appears.
  it("excludes H1 and H5, keeps H2-H4", () => {
    const doc = ["# H1", "## H2", "### H3", "#### H4", "##### H5", "body"].join("\n");
    const headings = extractHeadings(mkState(doc));
    expect(headings.map((h) => [h.level, h.text])).toEqual([
      [2, "H2"],
      [3, "H3"],
      [4, "H4"],
    ]);
  });

  // Failure mode 2: SetextHeading2 is missed.
  it("includes a setext (underline) H2, excludes a setext H1", () => {
    const doc = ["Title", "=====", "", "Sub", "-----", "", "body"].join("\n");
    const headings = extractHeadings(mkState(doc));
    expect(headings).toEqual([{ level: 2, text: "Sub", from: doc.indexOf("Sub") }]);
  });

  // Failure mode 3: a `## x` inside a fenced block is counted.
  it("does not count a heading-looking line inside a fenced code block", () => {
    const doc = ["## Real", "```", "## Fake", "```", "body"].join("\n");
    const headings = extractHeadings(mkState(doc));
    expect(headings.map((h) => h.text)).toEqual(["Real"]);
  });

  // Failure mode 4: inline marks (**, `, link URL) leak into the text.
  it("strips inline emphasis, code and link marks from the heading text", () => {
    const doc = "## Bold **text** and `code` and [link](http://example.com)";
    const headings = extractHeadings(mkState(doc));
    expect(headings).toEqual([{ level: 2, text: "Bold text and code and link", from: 0 }]);
  });

  // Failure mode 5: closing ## marks leak.
  it("strips a closing ATX mark sequence", () => {
    const doc = "## Heading ##\nbody";
    const headings = extractHeadings(mkState(doc));
    expect(headings).toEqual([{ level: 2, text: "Heading", from: 0 }]);
  });

  // Failure mode 6: an empty heading produces an empty item.
  it("skips a heading with no text", () => {
    const doc = "##\nbody";
    const headings = extractHeadings(mkState(doc));
    expect(headings).toEqual([]);
  });

  // Failure mode 7: `from` is not the heading line's start.
  it("uses the heading's line start for `from`, not the mark's position", () => {
    const doc = "  ## Indented\nbody";
    const headings = extractHeadings(mkState(doc));
    expect(headings).toEqual([{ level: 2, text: "Indented", from: 0 }]);
  });
});
