import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { EditorState } from "@codemirror/state";

const LEVEL: Record<string, 2 | 3 | 4> = { ATXHeading2: 2, ATXHeading3: 3, ATXHeading4: 4, SetextHeading2: 2 };
const MARKS = new Set(["HeaderMark", "EmphasisMark", "CodeMark", "LinkMark", "URL"]);

export type DocHeading = { level: 2 | 3 | 4; text: string; from: number };

function textWithoutMarks(
  state: EditorState,
  from: number,
  to: number,
  marks: { from: number; to: number }[],
): string {
  let text = "";
  let pos = from;
  for (const mark of marks) {
    text += state.doc.sliceString(pos, mark.from);
    pos = Math.max(pos, mark.to);
  }
  text += state.doc.sliceString(pos, to);
  return text;
}

export function extractHeadings(state: EditorState): DocHeading[] {
  const tree = ensureSyntaxTree(state, state.doc.length, 50) ?? syntaxTree(state);
  const out: DocHeading[] = [];
  tree.iterate({
    enter: (node) => {
      const level = LEVEL[node.name];
      if (!level) return;
      const line = state.doc.lineAt(node.from);
      const marks: { from: number; to: number }[] = [];
      tree.iterate({
        from: node.from,
        to: node.to,
        enter: (child) => {
          if (MARKS.has(child.name)) marks.push({ from: child.from, to: child.to });
        },
      });
      const text = textWithoutMarks(state, line.from, line.to, marks).trim();
      if (text) out.push({ level, text, from: line.from });
      return false;
    },
  });
  return out;
}
