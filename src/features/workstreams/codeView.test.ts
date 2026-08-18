import { unifiedMergeView } from "@codemirror/merge";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { buildDiffWordDeco } from "./codeView";

/** Build a unified merge state (the shape DiffTab feeds CodeMirror) and return
 *  the document slices that `buildDiffWordDeco` word-highlights. This exercises
 *  the real @codemirror/merge chunk output, so it guards the coordinate wiring
 *  (`getChunks` side + change offsets) end-to-end, not just `intraLineRanges`. */
function wordHighlightedSlices(original: string, modified: string): string[] {
  const state = EditorState.create({
    doc: modified,
    extensions: [unifiedMergeView({ original, highlightChanges: false })],
  });
  const deco = buildDiffWordDeco(state);
  const text = state.doc.toString();
  const slices: string[] = [];
  deco.between(0, text.length, (from, to) => {
    slices.push(text.slice(from, to));
  });
  return slices;
}

describe("buildDiffWordDeco", () => {
  it("highlights the changed word inside a modified line", () => {
    const slices = wordHighlightedSlices("const b = old", "const b = new");
    expect(slices.join("")).toContain("new");
    // Only the changed tail, never the unchanged "const b = " prefix.
    expect(slices.some((s) => s.includes("const b ="))).toBe(false);
  });

  it("does NOT highlight a wholly-new line (the double-highlight fix)", () => {
    // A modified word AND a brand-new line, @codemirror/merge lumps them into a
    // single change; only the word may be tinted, never the whole new line.
    const original = "const a = 1\nconst b = old\ndone()";
    const modified = "const a = 1\nconst b = new\nconst c = 3\ndone()";
    const slices = wordHighlightedSlices(original, modified);
    expect(slices).toContain("new");
    expect(slices.some((s) => s.includes("const c = 3"))).toBe(false);
  });

  it("emits no word highlights for a brand-new file (all pure insertions)", () => {
    expect(wordHighlightedSlices("", "line one\nline two\nline three")).toEqual([]);
  });

  it("emits no word highlights when nothing changed", () => {
    expect(wordHighlightedSlices("same\ntext", "same\ntext")).toEqual([]);
  });
});
