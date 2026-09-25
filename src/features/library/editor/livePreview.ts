import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { EditorState } from "@codemirror/state";
import { RangeSetBuilder } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";

const HIDE = new Set(["HeaderMark", "EmphasisMark", "CodeMark", "LinkMark", "URL", "StrikethroughMark"]);

export function hiddenRanges(state: EditorState, visible: readonly { from: number; to: number }[]) {
  const out: { from: number; to: number }[] = [];
  // Forces the parse up to the viewport rather than reading whatever's parsed
  // so far, so the first paint doesn't flash raw marks before the background
  // parser catches up (same budget headings.ts uses for the same reason).
  const parseTo = visible.reduce((max, r) => Math.max(max, r.to), 0);
  const tree = ensureSyntaxTree(state, parseTo, 50) ?? syntaxTree(state);
  for (const { from, to } of visible) {
    tree.iterate({
      from,
      to,
      enter: (node) => {
        if (!HIDE.has(node.name)) return;
        const parent = node.node.parent;
        if (!parent) return;
        if (node.name === "HeaderMark" && !parent.name.startsWith("ATXHeading")) return; // setext underline stays
        if (node.name === "CodeMark" && parent.name !== "InlineCode") return; // fences stay
        if ((node.name === "LinkMark" || node.name === "URL") && parent.name !== "Link") return; // images, autolinks stay
        const lineFrom = state.doc.lineAt(parent.from).from;
        const lineTo = state.doc.lineAt(parent.to).to;
        if (state.selection.ranges.some((r) => r.to >= lineFrom && r.from <= lineTo)) return;
        const end =
          node.name === "HeaderMark" && state.doc.sliceString(node.to, node.to + 1) === " " ? node.to + 1 : node.to;
        out.push({ from: node.from, to: end });
      },
    });
  }
  return dedupeSorted(out);
}

// Merges overlapping ranges into sorted, disjoint ones, which RangeSetBuilder requires.
function dedupeSorted(ranges: { from: number; to: number }[]) {
  const sorted = [...ranges].sort((a, b) => a.from - b.from || a.to - b.to);
  const out: { from: number; to: number }[] = [];
  for (const r of sorted) {
    const last = out[out.length - 1];
    if (last && r.from <= last.to) last.to = Math.max(last.to, r.to);
    else out.push({ ...r });
  }
  return out;
}

const HIDDEN = Decoration.replace({});

function build(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of hiddenRanges(view.state, view.visibleRanges)) builder.add(from, to, HIDDEN);
  return builder.finish();
}

export const livePreview = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = build(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.selectionSet || u.viewportChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) {
        this.decorations = build(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);
