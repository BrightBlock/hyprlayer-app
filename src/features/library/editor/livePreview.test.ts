import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { ensureSyntaxTree } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorState, RangeSetBuilder } from "@codemirror/state";
import { Decoration } from "@codemirror/view";
import { describe, expect, it } from "vitest";

import { hiddenRanges } from "./livePreview";

function mkState(doc: string, selection?: { anchor: number; head?: number }): EditorState {
  const state = EditorState.create({
    doc,
    selection,
    extensions: [markdown({ base: markdownLanguage, codeLanguages: languages })],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  return state;
}

// Applies the same hiding a Decoration.replace would, so assertions read as plain text.
function rendered(state: EditorState, visible: { from: number; to: number }[] = [{ from: 0, to: state.doc.length }]) {
  const ranges = hiddenRanges(state, visible);
  let text = state.doc.toString();
  for (let i = ranges.length - 1; i >= 0; i--) text = text.slice(0, ranges[i].from) + text.slice(ranges[i].to);
  return text;
}

describe("hiddenRanges", () => {
  // Failure mode 1: **, *, `, # or link brackets and URL stay visible on a line the
  // cursor is not on.
  it("hides bold, italic, code, heading and link marks on lines the cursor is not on", () => {
    const lines = ["# Heading", "**bold**", "*italic*", "`code`", "[text](url)", "plain"];
    const doc = lines.join("\n");
    const state = mkState(doc, { anchor: doc.indexOf("plain") });
    expect(rendered(state)).toBe(["Heading", "bold", "italic", "code", "text", "plain"].join("\n"));
  });

  // Failure mode 2: marks are hidden on the cursor's line.
  it("reveals marks on the cursor's line", () => {
    const doc = ["plain before", "**bold**", "plain after"].join("\n");
    const state = mkState(doc, { anchor: doc.indexOf("bold") });
    expect(rendered(state)).toBe(doc);
  });

  // Failure mode 3: a selection spanning two lines leaves either line hidden.
  it("reveals marks on every line a multi-line selection touches", () => {
    const doc = ["**bold**", "*italic*"].join("\n");
    const state = mkState(doc, { anchor: 2, head: doc.length - 2 });
    expect(rendered(state)).toBe(doc);
  });

  // Failure mode 4: the space after ## stays behind, so the text is indented.
  it("eats the space after a hidden ATX heading mark", () => {
    const doc = "## Heading\nplain";
    const state = mkState(doc, { anchor: doc.indexOf("plain") });
    expect(rendered(state)).toBe("Heading\nplain");
  });

  // Failure mode 5: fenced-code fences or a setext underline get hidden.
  it("never hides fenced-code fences or a setext underline", () => {
    const doc = ["```ts", "code here", "```", "Setext", "===", "plain"].join("\n");
    const state = mkState(doc, { anchor: doc.indexOf("plain") });
    expect(rendered(state)).toBe(doc);
  });

  // Failure mode 6: an autolink's or bare URL's text gets hidden.
  it("never hides an autolink or a bare URL", () => {
    const doc = ["<http://example.com>", "http://example.com", "plain"].join("\n");
    const state = mkState(doc, { anchor: doc.indexOf("plain") });
    expect(rendered(state)).toBe(doc);
  });

  // Failure mode 7: the ranges overlap or are unsorted, which makes RangeSetBuilder throw.
  it("returns sorted, non-overlapping ranges that RangeSetBuilder accepts", () => {
    const doc = "## H2 with `code` and **bold**\nplain text here";
    const state = mkState(doc, { anchor: doc.indexOf("plain") });
    const ranges = hiddenRanges(state, [{ from: 0, to: state.doc.length }]);
    for (let i = 1; i < ranges.length; i++) {
      expect(ranges[i].from).toBeGreaterThanOrEqual(ranges[i - 1].to);
    }
    expect(() => {
      const builder = new RangeSetBuilder<Decoration>();
      for (const r of ranges) builder.add(r.from, r.to, Decoration.replace({}));
      builder.finish();
    }).not.toThrow();
  });

  // Failure mode 8: the ranges change the document text; doc.toString() must be unchanged.
  it("never mutates the document", () => {
    const doc = "**bold**\nplain";
    const state = mkState(doc, { anchor: 0 });
    const before = state.doc.toString();
    hiddenRanges(state, [{ from: 0, to: state.doc.length }]);
    expect(state.doc.toString()).toBe(before);
  });

  // Failure mode 9: an image loses its brackets but keeps its URL, rendering as "alturl".
  it("never hides image marks", () => {
    const doc = "![alt](url)\nplain";
    const state = mkState(doc, { anchor: doc.indexOf("plain") });
    expect(rendered(state)).toBe(doc);
  });

  // Failure mode 10: ~~strike~~ keeps its ~~ off-cursor.
  it("hides strikethrough marks off-cursor and reveals them on-cursor", () => {
    const doc = "~~strike~~\nplain";
    const offCursor = mkState(doc, { anchor: doc.indexOf("plain") });
    expect(rendered(offCursor)).toBe("strike\nplain");
    const onCursor = mkState(doc, { anchor: doc.indexOf("strike") });
    expect(rendered(onCursor)).toBe(doc);
  });
});
