import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import { RangeSetBuilder } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";

// Line decorations, not a block widget (the plan rules out block widgets in
// live preview) — a background/border on the fence's own lines, matching the
// bg-panel/border-edge/rounded-md container ShikiCodeBlock uses in the
// read-only preview, so a fence reads as a block instead of plain colored text.
const LINE = Decoration.line({ class: "cm-fenced-code" });
const LINE_START = Decoration.line({ class: "cm-fenced-code cm-fenced-code-start" });
const LINE_END = Decoration.line({ class: "cm-fenced-code cm-fenced-code-end" });
const LINE_SOLO = Decoration.line({ class: "cm-fenced-code cm-fenced-code-start cm-fenced-code-end" });

export const fencedCodeTheme = EditorView.theme({
  ".cm-fenced-code": {
    backgroundColor: "var(--color-panel)",
    borderLeft: "1px solid var(--color-edge)",
    borderRight: "1px solid var(--color-edge)",
    paddingLeft: "12px",
    paddingRight: "12px",
    // A fence with a recognized language mounts that language's own grammar
    // inside it, so its tokens carry that language's tags (keyword, string,
    // ...), not markdown's CodeText/InlineCode — the only tags the base
    // markdownHighlight style hands var(--font-mono) to. Set it here instead,
    // on the block itself, so it applies regardless of which tags land inside.
    fontFamily: "var(--font-mono)",
    fontSize: "var(--text-meta)",
    lineHeight: "var(--text-meta--line-height)",
  },
  ".cm-fenced-code-start": {
    borderTop: "1px solid var(--color-edge)",
    borderTopLeftRadius: "var(--radius-card)",
    borderTopRightRadius: "var(--radius-card)",
    marginTop: "8px",
    paddingTop: "6px",
  },
  ".cm-fenced-code-end": {
    borderBottom: "1px solid var(--color-edge)",
    borderBottomLeftRadius: "var(--radius-card)",
    borderBottomRightRadius: "var(--radius-card)",
    marginBottom: "8px",
    paddingBottom: "6px",
  },
});

// Drops exact-duplicate blocks and sorts by `from`, which RangeSetBuilder requires.
// Distinct FencedCode nodes never partially overlap, so exact-match dedupe is enough.
function dedupeSorted(blocks: { from: number; to: number }[]) {
  const sorted = [...blocks].sort((a, b) => a.from - b.from);
  const out: { from: number; to: number }[] = [];
  for (const b of sorted) {
    const last = out[out.length - 1];
    if (last && b.from === last.from && b.to === last.to) continue;
    out.push(b);
  }
  return out;
}

function build(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const parseTo = view.visibleRanges.reduce((max, r) => Math.max(max, r.to), 0);
  // Same reasoning as livePreview.ts: force the parse for the visible area
  // instead of reading whatever's parsed so far, so the block doesn't paint
  // without its background for a beat after the fence itself appears.
  const tree = ensureSyntaxTree(view.state, parseTo, 50) ?? syntaxTree(view.state);

  const blocks: { from: number; to: number }[] = [];
  for (const { from, to } of view.visibleRanges) {
    tree.iterate({
      from,
      to,
      enter: (node) => {
        if (node.name === "FencedCode") blocks.push({ from: node.from, to: node.to });
      },
    });
  }

  // A block straddling a fold boundary (or any other visibleRanges split) gets
  // visited once per overlapping range; same fix as livePreview.ts's
  // dedupeSorted, needed here because RangeSetBuilder requires ascending `from`.
  for (const block of dedupeSorted(blocks)) {
    const firstLine = view.state.doc.lineAt(block.from).number;
    const lastLine = view.state.doc.lineAt(block.to).number;
    for (let n = firstLine; n <= lastLine; n++) {
      const line = view.state.doc.line(n);
      const deco =
        firstLine === lastLine ? LINE_SOLO : n === firstLine ? LINE_START : n === lastLine ? LINE_END : LINE;
      builder.add(line.from, line.from, deco);
    }
  }
  return builder.finish();
}

export const fencedCodeBlocks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = build(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) {
        this.decorations = build(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);
